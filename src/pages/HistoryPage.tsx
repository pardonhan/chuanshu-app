import { Card, Table, Button, Space, message, Modal } from 'antd'
import { FolderOutlined, DeleteOutlined, RedoOutlined } from '@ant-design/icons'
import { useEffect, useState } from 'react'
import { getTransferHistory, clearTransferHistory, type TransferHistoryEntry } from '../services/tauriApi'

interface TableItem {
  key: string
  task_id: string
  fileNames: string
  fileSize: string
  type: 'send' | 'receive'
  status: 'success' | 'failed' | 'canceled'
  date: string
  device: string
  raw: TransferHistoryEntry
}

const formatFileSize = (bytes: number): string => {
  if (bytes === 0) return '0 B'
  const k = 1024
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.floor(Math.log(bytes) / Math.log(k))
  return Math.round(bytes / Math.pow(k, i) * 100) / 100 + ' ' + sizes[i]
}

const formatTimestamp = (timestamp: number): string => {
  const date = new Date(timestamp * 1000)
  return date.toLocaleString('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit'
  })
}

const convertToTableItem = (entry: TransferHistoryEntry): TableItem => {
  const fileNames = entry.file_names || `文件 (${entry.file_count} 个)`
  const statusMap: Record<string, 'success' | 'failed' | 'canceled'> = {
    completed: 'success',
    failed: 'failed',
    canceled: 'canceled',
  }

  return {
    key: entry.task_id,
    task_id: entry.task_id,
    fileNames,
    fileSize: formatFileSize(entry.total_size),
    type: entry.transfer_type as 'send' | 'receive',
    status: statusMap[entry.status] || 'canceled',
    date: formatTimestamp(entry.created_at),
    device: entry.peer_device_name,
    raw: entry,
  }
}

const columns: any[] = [
  {
    title: '文件名',
    dataIndex: 'fileNames',
    key: 'fileNames',
  },
  {
    title: '大小',
    dataIndex: 'fileSize',
    key: 'fileSize',
    width: 100,
  },
  {
    title: '类型',
    dataIndex: 'type',
    key: 'type',
    width: 80,
    render: (type: string) => type === 'send' ? '发送' : '接收',
  },
  {
    title: '状态',
    dataIndex: 'status',
    key: 'status',
    width: 80,
    render: (status: string) => {
      const statusMap: Record<string, { color: string; text: string }> = {
        success: { color: 'green', text: '成功' },
        failed: { color: 'red', text: '失败' },
        canceled: { color: 'orange', text: '已取消' },
      }
      const info = statusMap[status] || { color: 'gray', text: '未知' }
      return <span style={{ color: info.color }}>{info.text}</span>
    },
  },
  {
    title: '设备',
    dataIndex: 'device',
    key: 'device',
    width: 120,
  },
  {
    title: '时间',
    dataIndex: 'date',
    key: 'date',
    width: 160,
  },
  {
    title: '操作',
    key: 'action',
    width: 150,
  },
]

export default function HistoryPage() {
  const [data, setData] = useState<TableItem[]>([])
  const [loading, setLoading] = useState(false)

  const loadHistory = async () => {
    setLoading(true)
    try {
      const entries = await getTransferHistory({ limit: 100, offset: 0 })
      setData(entries.map(convertToTableItem))
    } catch (error) {
      message.error('加载历史记录失败')
      console.error('Failed to load transfer history:', error)
    } finally {
      setLoading(false)
    }
  }

  const handleClearHistory = async () => {
    Modal.confirm({
      title: '确认清空历史记录？',
      content: '清空后将无法恢复，确定要继续吗？',
      okText: '确定',
      cancelText: '取消',
      okType: 'danger',
      onOk: async () => {
        try {
          await clearTransferHistory()
          message.success('历史记录已清空')
          loadHistory()
        } catch (error) {
          message.error('清空历史记录失败')
          console.error('Failed to clear history:', error)
        }
      },
    })
  }

  const handleOpenLocation = async (_record: TableItem) => {
    // For completed receive transfers, we could store the download path
    // For now, show a message as we don't have the actual path stored
    message.info('文件路径需要在历史记录中保存')
    // TODO: Once we have file_path in TransferHistoryEntry, call:
    // await openFileLocation(record.filePath)
  }

  const handleResend = async (_record: TableItem) => {
    // TODO: This would need the original file paths to resend
    // For now, show a message
    message.info('重新发送功能需要原始文件路径')
  }

  const columnsWithHandlers = columns.map(col => {
    if (col.key === 'action') {
      return {
        ...col,
        render: (_: unknown, record: TableItem) => (
          <Space size={4}>
            <Button
              type="text"
              icon={<FolderOutlined />}
              title="打开位置"
              onClick={() => handleOpenLocation(record)}
            />
            <Button
              type="text"
              icon={<RedoOutlined />}
              title="重新发送"
              disabled={record.type !== 'send'}
              onClick={() => handleResend(record)}
            />
          </Space>
        ),
      }
    }
    return col
  })

  useEffect(() => {
    loadHistory()
  }, [])

  return (
    <Card
      title="传输历史"
      extra={
        <Button
          danger
          icon={<DeleteOutlined />}
          onClick={handleClearHistory}
          disabled={data.length === 0}
        >
          清空历史
        </Button>
      }
    >
      <Table
        columns={columnsWithHandlers}
        dataSource={data}
        rowKey="key"
        pagination={{ pageSize: 20, showSizeChanger: false }}
        loading={loading}
        locale={{ emptyText: '暂无历史记录' }}
      />
    </Card>
  )
}
