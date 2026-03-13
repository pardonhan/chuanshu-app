import { ConfigProvider, theme } from 'antd'
import zhCN from 'antd/locale/zh_CN'
import { useState, useEffect } from 'react'
import { Layout } from 'antd'
import Sidebar from './components/layout/Sidebar'
import HomePage from './pages/HomePage'
import HistoryPage from './pages/HistoryPage'
import SettingsPage from './pages/SettingsPage'
import { getSettings } from './services/tauriApi'

const { Sider, Content } = Layout

type Page = 'home' | 'history' | 'settings'

function App() {
  const [currentPage, setCurrentPage] = useState<Page>('home')
  const [currentTheme, setCurrentTheme] = useState<'light' | 'dark'>('light')

  // Load theme on mount
  useEffect(() => {
    const loadTheme = async () => {
      try {
        const settings = await getSettings()
        applyTheme(settings.theme)
      } catch (error) {
        console.error('Failed to load theme:', error)
      }
    }
    loadTheme()

    // Listen for system theme changes
    const mediaQuery = window.matchMedia('(prefers-color-scheme: dark)')
    const handleChange = () => {
      getSettings().then(settings => {
        if (settings.theme === 'auto') {
          applyTheme('auto')
        }
      })
    }
    mediaQuery.addEventListener('change', handleChange)
    return () => mediaQuery.removeEventListener('change', handleChange)
  }, [])

  const applyTheme = (themeSetting: string) => {
    if (themeSetting === 'auto') {
      const isDark = window.matchMedia('(prefers-color-scheme: dark)').matches
      setCurrentTheme(isDark ? 'dark' : 'light')
    } else {
      setCurrentTheme(themeSetting as 'light' | 'dark')
    }
  }

  // Handle theme change from settings
  useEffect(() => {
    const handleThemeChange = () => {
      getSettings().then(settings => applyTheme(settings.theme))
    }
    // You can add custom event listener here if needed
    handleThemeChange()
  }, [currentPage])

  return (
    <ConfigProvider
      locale={zhCN}
      theme={{
        token: {
          colorPrimary: '#1890ff',
          borderRadius: 6,
        },
        algorithm: currentTheme === 'dark' ? theme.darkAlgorithm : theme.defaultAlgorithm,
      }}
    >
      <Layout style={{ height: '100vh' }}>
        <Sider
          width={200}
          theme={currentTheme === 'dark' ? 'dark' : 'light'}
          style={{
            background: currentTheme === 'dark' ? '#141414' : '#ffffff',
          }}
        >
          <Sidebar currentPage={currentPage} onPageChange={setCurrentPage} />
        </Sider>
        <Layout>
          <Content
            style={{
              padding: '24px',
              overflow: 'auto',
              background: currentTheme === 'dark' ? '#1f1f1f' : '#f5f5f5',
            }}
          >
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
