import { ConfigProvider } from 'antd'
import zhCN from 'antd/locale/zh_CN'
import { useState } from 'react'
import { Layout } from 'antd'
import Sidebar from './components/layout/Sidebar'
import HomePage from './pages/HomePage'
import HistoryPage from './pages/HistoryPage'
import SettingsPage from './pages/SettingsPage'

const { Sider, Content } = Layout

type Page = 'home' | 'history' | 'settings'

function App() {
  const [currentPage, setCurrentPage] = useState<Page>('home')

  return (
    <ConfigProvider locale={zhCN} theme={{
      token: {
        colorPrimary: '#1890ff',
      },
    }}>
      <Layout style={{ height: '100vh' }}>
        <Sider width={200} theme="light">
          <Sidebar currentPage={currentPage} onPageChange={setCurrentPage} />
        </Sider>
        <Layout>
          <Content style={{ padding: '24px', overflow: 'auto' }}>
            {currentPage === 'home' && <HomePage />}
            {currentPage === 'history' && <HistoryPage />}
            {currentPage === 'settings' && <SettingsPage />}
          </Content>
        </Layout>
      </Layout>
    </ConfigProvider>
  )
}

export default App
