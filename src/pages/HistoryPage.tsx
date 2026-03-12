import { Card, Table, Button, Space, Typography, Empty, message } from 'antd'
import { FolderOutlined, DeleteOutlined, RedoOutlined } from '@ant-design/icons'
import { useEffect, useState } from 'react'
import { getTransferHistory, clearTransferHistory, type TransferHistoryEntry } from '../services/tauriApi'

const { Text } = Typography

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

const columns = [
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
    render: (_: unknown, record: TableItem) => (
      <Space size={4}>
        <Button type="text" icon={<FolderOutlined />} title="打开位置" />
        <Button type="text" icon={<RedoOutlined />} title="重新发送" />
      </Space>
    ),
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
    try {
      await clearTransferHistory()
      message.success('历史记录已清空')
      loadHistory()
    } catch (error) {
      message.error('清空历史记录失败')
      console.error('Failed to clear history:', error)
    }
  }

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
        columns={columns}
        dataSource={data}
        rowKey="key"
        pagination={{ pageSize: 20, showSizeChanger: false }}
        loading={loading}
        locale={{ emptyText: '暂无历史记录' }}
      />
    </Card>
  )
}
