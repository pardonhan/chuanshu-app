import { Card, Table, Button, Space } from 'antd'
import { FolderOpenOutlined, RedoOutlined } from '@ant-design/icons'

interface HistoryItem {
  id: string
  fileName: string
  fileSize: string
  type: 'send' | 'receive'
  status: 'success' | 'failed' | 'canceled'
  date: string
  device: string
}

const mockData: HistoryItem[] = [
  {
    id: '1',
    fileName: '项目文档.zip',
    fileSize: '1.2 GB',
    type: 'send',
    status: 'success',
    date: '2026-03-11 15:30',
    device: 'MacBook Pro'
  },
  {
    id: '2',
    fileName: '照片集',
    fileSize: '4.5 GB',
    type: 'receive',
    status: 'success',
    date: '2026-03-10 18:20',
    device: 'Windows PC'
  },
]

const columns = [
  {
    title: '文件名',
    dataIndex: 'fileName',
    key: 'fileName',
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
    width: 120,
    render: () => (
      <Space>
        <Button type="text" icon={<FolderOpenOutlined />}>打开位置</Button>
        <Button type="text" icon={<RedoOutlined />}>重新发送</Button>
      </Space>
    ),
  },
]

export default function HistoryPage() {
  return (
    <Card title="传输历史">
      <Table
        columns={columns}
        dataSource={mockData}
        rowKey="id"
        pagination={{ pageSize: 20 }}
      />
    </Card>
  )
}
