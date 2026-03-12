import { Menu } from 'antd'
import { FileSyncOutlined, HistoryOutlined, SettingOutlined } from '@ant-design/icons'
import type { MenuProps } from 'antd'

type Page = 'home' | 'history' | 'settings'

interface SidebarProps {
  currentPage: Page
  onPageChange: (page: Page) => void
}

type MenuItem = Required<MenuProps>['items'][number]

const items: MenuItem[] = [
  {
    key: 'home',
    icon: <FileSyncOutlined />,
    label: '文件传输',
  },
  {
    key: 'history',
    icon: <HistoryOutlined />,
    label: '传输历史',
  },
  {
    key: 'settings',
    icon: <SettingOutlined />,
    label: '设置',
  },
]

export default function Sidebar({ currentPage, onPageChange }: SidebarProps) {
  return (
    <div style={{ padding: '16px 0' }}>
      <div style={{
        fontSize: '20px',
        fontWeight: 'bold',
        textAlign: 'center',
        marginBottom: '24px',
        color: '#1890ff'
      }}>
        传书
      </div>
      <Menu
        mode="inline"
        selectedKeys={[currentPage]}
        items={items}
        onClick={({ key }) => onPageChange(key as Page)}
        style={{ border: 'none' }}
      />
    </div>
  )
}
