import { useEffect, useCallback } from 'react'
import { List, Progress, Button, Space, Tag, Empty } from 'antd'
import { PauseOutlined, PlayCircleOutlined, CloseOutlined, FolderOpenOutlined } from '@ant-design/icons'
import { useTransferStore } from '../../store/useTransferStore'
import {
  getTransferTasks,
  cancelTransfer,
  pauseTransfer,
  resumeTransfer,
  onTransferProgress,
  onTransferCompleted,
  onTransferFailed,
  type TransferTaskInfo
} from '../../services/tauriApi'
import type { UnlistenFn } from '@tauri-apps/api/event'
import { filesize } from 'filesize'

export default function TransferList() {
  const {
    tasks,
    isLoading,
    setTasks,
    updateTaskProgress,
    setTaskStatus,
    setLoading
  } = useTransferStore()

  // Load tasks on mount
  const loadTasks = useCallback(async () => {
    setLoading(true)
    try {
      const taskList = await getTransferTasks()
      setTasks(taskList)
    } catch (error) {
      console.error('Failed to load tasks:', error)
    } finally {
      setLoading(false)
    }
  }, [setTasks, setLoading])

  useEffect(() => {
    loadTasks()

    // Set up event listeners
    let unlistenProgress: UnlistenFn | null = null
    let unlistenCompleted: UnlistenFn | null = null
    let unlistenFailed: UnlistenFn | null = null

    const setupListeners = async () => {
      unlistenProgress = await onTransferProgress((task: TransferTaskInfo) => {
        updateTaskProgress(task.task_id, task.progress, task.speed, task.transferred_size)
      })

      unlistenCompleted = await onTransferCompleted((taskId: string) => {
        setTaskStatus(taskId, 'Completed')
        loadTasks()
      })

      unlistenFailed = await onTransferFailed(({ taskId }: { taskId: string; error: string }) => {
        setTaskStatus(taskId, 'Failed')
        loadTasks()
      })
    }

    setupListeners()

    // Poll for updates every 2 seconds
    const interval = setInterval(loadTasks, 2000)

    return () => {
      clearInterval(interval)
      unlistenProgress?.()
      unlistenCompleted?.()
      unlistenFailed?.()
    }
  }, [loadTasks, updateTaskProgress, setTaskStatus])

  const handlePause = async (taskId: string) => {
    try {
      await pauseTransfer(taskId)
      setTaskStatus(taskId, 'Paused')
    } catch (error) {
      console.error('Failed to pause transfer:', error)
    }
  }

  const handleResume = async (taskId: string) => {
    try {
      await resumeTransfer(taskId)
      setTaskStatus(taskId, 'Transferring')
    } catch (error) {
      console.error('Failed to resume transfer:', error)
    }
  }

  const handleCancel = async (taskId: string) => {
    try {
      await cancelTransfer(taskId)
      setTaskStatus(taskId, 'Canceled')
    } catch (error) {
      console.error('Failed to cancel transfer:', error)
    }
  }

  const getStatusText = (status: string) => {
    const statusMap: Record<string, { color: string; text: string }> = {
      Pending: { color: 'default', text: '等待中' },
      Transferring: { color: 'blue', text: '传输中' },
      Paused: { color: 'orange', text: '已暂停' },
      Completed: { color: 'success', text: '已完成' },
      Failed: { color: 'error', text: '失败' },
      Canceled: { color: 'default', text: '已取消' },
    }
    return statusMap[status] || { color: 'default', text: '未知' }
  }

  const formatSpeed = (bytesPerSecond: number) => {
    if (bytesPerSecond === 0) return ''
    return filesize(bytesPerSecond, { base: 2 }) + '/s'
  }

  return (
    <div>
      {tasks.length === 0 ? (
        <Empty
          image={Empty.PRESENTED_IMAGE_SIMPLE}
          description="暂无传输任务"
          style={{ marginTop: 40 }}
        />
      ) : (
        <List
          dataSource={tasks}
          loading={isLoading}
          renderItem={(task) => (
            <List.Item
              key={task.task_id}
              actions={[
                task.status === 'Transferring' ? (
                  <Button
                    type="text"
                    icon={<PauseOutlined />}
                    size="small"
                    onClick={() => handlePause(task.task_id)}
                  />
                ) : task.status === 'Paused' ? (
                  <Button
                    type="text"
                    icon={<PlayCircleOutlined />}
                    size="small"
                    onClick={() => handleResume(task.task_id)}
                  />
                ) : null,
                task.status !== 'Completed' && task.status !== 'Canceled' && (
                  <Button
                    type="text"
                    icon={<CloseOutlined />}
                    size="small"
                    danger
                    onClick={() => handleCancel(task.task_id)}
                  />
                ),
                task.status === 'Completed' && (
                  <Button type="text" icon={<FolderOpenOutlined />} size="small" />
                ),
              ].filter(Boolean)}
            >
              <List.Item.Meta
                title={
                  <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 8 }}>
                    <span>{task.current_file || `${task.file_count} 个文件`}</span>
                    <Space>
                      <Tag color={getStatusText(task.status).color}>
                        {getStatusText(task.status).text}
                      </Tag>
                      <Tag color={task.transfer_type === 'Send' ? 'blue' : 'green'}>
                        {task.transfer_type === 'Send' ? '发送到' : '来自'} {task.peer_device_name}
                      </Tag>
                    </Space>
                  </div>
                }
                description={
                  <div>
                    <div style={{ marginBottom: 8 }}>
                      <Progress
                        percent={Math.round(task.progress * 100) / 100}
                        size="small"
                        status={task.status === 'Failed' ? 'exception' : undefined}
                      />
                    </div>
                    <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 12, color: '#666' }}>
                      <span>
                        {filesize(task.transferred_size, { base: 2 })} / {filesize(task.total_size, { base: 2 })}
                      </span>
                      {task.status === 'Transferring' && (
                        <span>{formatSpeed(task.speed)}</span>
                      )}
                    </div>
                    {task.error_message && (
                      <div style={{ fontSize: 12, color: '#ff4d4f', marginTop: 4 }}>
                        错误: {task.error_message}
                      </div>
                    )}
                  </div>
                }
              />
            </List.Item>
          )}
        />
      )}
    </div>
  )
}
