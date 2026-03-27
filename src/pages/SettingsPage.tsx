import { useEffect, useState } from 'react'
import { Card, Form, Input, Switch, Select, Button, Space, Divider, message, Row, Col, Slider } from 'antd'
import { SaveOutlined, FolderOutlined, ReloadOutlined } from '@ant-design/icons'
import { getSettings, saveSettings, type Settings, getSystemInfo, type SystemInfo, getAutoLaunch, setAutoLaunch as setAutoLaunchApi } from '../services/tauriApi'
import { open } from '@tauri-apps/plugin-dialog'

const { Option } = Select

// Apply theme based on setting and dispatch custom event for App.tsx to listen
const applyTheme = (theme: 'auto' | 'light' | 'dark') => {
  const root = document.documentElement
  if (theme === 'light') {
    root.removeAttribute('data-theme')
    root.style.colorScheme = 'light'
  } else if (theme === 'dark') {
    root.setAttribute('data-theme', 'dark')
    root.style.colorScheme = 'dark'
  } else {
    // Auto: follow system preference
    root.removeAttribute('data-theme')
    root.style.colorScheme = ''
  }
  localStorage.setItem('theme', theme)

  // Dispatch custom event for App.tsx to listen and update its state
  window.dispatchEvent(new CustomEvent('theme-change', { detail: { theme } }))
}

// Slider marks with compact labels
const speedMarks = {
  0: '不限',
  10: '10MB',
  50: '50MB',
  100: '100MB',
}

export default function SettingsPage() {
  const [form] = Form.useForm()
  const [isLoading, setIsLoading] = useState(false)
  const [isSaving, setIsSaving] = useState(false)
  const [systemInfo, setSystemInfo] = useState<SystemInfo | null>(null)
  const [autoLaunch, setAutoLaunch] = useState(false)

  useEffect(() => {
    loadSystemInfo()
    loadAutoLaunch()
    loadSettings()
  }, [])

  const loadSystemInfo = async () => {
    try {
      const info = await getSystemInfo()
      setSystemInfo(info)
    } catch (error) {
      console.error('Failed to load system info:', error)
    }
  }

  const loadAutoLaunch = async () => {
    try {
      const enabled = await getAutoLaunch()
      setAutoLaunch(enabled)
    } catch (error) {
      console.error('Failed to load auto launch status:', error)
    }
  }

  const loadSettings = async () => {
    setIsLoading(true)
    try {
      const settings = await getSettings()
      form.setFieldsValue(settings)
      // Apply theme setting
      applyTheme(settings.theme as 'auto' | 'light' | 'dark')
    } catch (error) {
      console.error('Failed to load settings:', error)
      message.error('加载设置失败')
    } finally {
      setIsLoading(false)
    }
  }

  // Apply theme on mount and when form values change
  const themeValue = Form.useWatch('theme', form)
  useEffect(() => {
    if (themeValue) {
      applyTheme(themeValue as 'auto' | 'light' | 'dark')
    }
  }, [themeValue])

  const onFinish = async (values: Settings) => {
    setIsSaving(true)
    try {
      await saveSettings(values)
      // Apply theme after saving
      applyTheme(values.theme as 'auto' | 'light' | 'dark')
      message.success('设置已保存')
    } catch (error) {
      console.error('Failed to save settings:', error)
      message.error('保存设置失败')
    } finally {
      setIsSaving(false)
    }
  }

  const handleSelectDownloadPath = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: '选择下载文件夹',
      })
      if (selected) {
        form.setFieldValue('download_path', selected as string)
      }
    } catch (error) {
      console.error('Failed to select download path:', error)
      message.error('选择文件夹失败')
    }
  }

  const handleAutoLaunchChange = async (checked: boolean) => {
    try {
      await setAutoLaunchApi(checked)
      setAutoLaunch(checked)
      message.success(checked ? '已启用开机自启' : '已禁用开机自启')
    } catch (error) {
      console.error('Failed to set auto launch:', error)
      message.error('设置开机自启失败')
      setAutoLaunch(!checked)
    }
  }

  const getSystemLabel = () => {
    if (!systemInfo) return '检测中...'
    const osLabel = systemInfo.os === 'windows' ? 'Windows' : systemInfo.os === 'macos' ? 'macOS' : 'Linux'
    return `${osLabel} ${systemInfo.os_version} (${systemInfo.arch})`
  }

  return (
    <Card
      title="设置"
      loading={isLoading}
      extra={
        <Space>
          <Button icon={<ReloadOutlined />} size="small" onClick={loadSettings}>刷新</Button>
        </Space>
      }
    >
      <Form
        form={form}
        layout="vertical"
        onFinish={onFinish}
        style={{ maxWidth: 600 }}
        initialValues={{
          upload_limit: 0,
          download_limit: 0,
          auto_accept: false,
          enable_notification: true,
          theme: 'auto',
        }}
      >
        <div style={{ marginBottom: 16, fontSize: 16, fontWeight: 500, borderBottom: '1px solid #e8e8e8', paddingBottom: 8 }}>
          基本设置
        </div>

        <Form.Item
          label="设备名称"
          name="device_name"
          rules={[{ required: true, message: '请输入设备名称' }]}
          extra={<span style={{ color: '#999', fontSize: '12px' }}>设备名称将在局域网中显示，用于标识本设备</span>}
        >
          <Input
            placeholder="输入设备在局域网中显示的名称"
            maxLength={50}
            showCount
            value={Form.useWatch('device_name', form)}
            onChange={(e) => form.setFieldValue('device_name', e.target.value)}
          />
        </Form.Item>

        <Form.Item
          label="默认下载路径"
          name="download_path"
          rules={[{ required: true, message: '请输入下载路径' }]}
          extra={<span style={{ color: '#999', fontSize: '12px' }}>系统推荐路径：{systemInfo?.default_download_path || '检测中...'}</span>}
        >
          <Input.Group compact>
            <Input
              style={{ width: 'calc(100% - 100px)' }}
              placeholder="文件接收后的保存路径"
              value={Form.useWatch('download_path', form)}
              onChange={(e) => form.setFieldValue('download_path', e.target.value)}
            />
            <Button icon={<FolderOutlined />} onClick={handleSelectDownloadPath}>
              浏览
            </Button>
          </Input.Group>
        </Form.Item>

        <div style={{ marginBottom: 16, fontSize: 16, fontWeight: 500, borderBottom: '1px solid #e8e8e8', paddingBottom: 8 }}>
          系统设置
        </div>

        <Form.Item label="开机自启" extra={<span style={{ color: '#999', fontSize: '12px' }}>启用后应用将随系统自动启动</span>}>
          <Switch checked={autoLaunch} onChange={handleAutoLaunchChange} />
        </Form.Item>

        <Form.Item label="系统信息" extra={<span style={{ color: '#999', fontSize: '12px' }}>{getSystemLabel()}</span>}>
          <Input disabled value={getSystemLabel()} />
        </Form.Item>

        <div style={{ marginBottom: 16, fontSize: 16, fontWeight: 500, borderBottom: '1px solid #e8e8e8', paddingBottom: 8 }}>
          传输设置
        </div>

        <Form.Item
          label="自动接收文件"
          name="auto_accept"
          valuePropName="checked"
          extra="启用后收到的文件将自动保存到下载路径"
        >
          <Switch />
        </Form.Item>

        <Form.Item
          label="上传限速"
          name="upload_limit"
          extra="限制上传速度，避免占用全部带宽（单位：MB/s）"
        >
          <Slider
            min={0}
            max={100}
            marks={speedMarks}
            step={1}
          />
        </Form.Item>

        <Form.Item
          label="下载限速"
          name="download_limit"
          extra="限制下载速度，避免占用全部带宽（单位：MB/s）"
        >
          <Slider
            min={0}
            max={100}
            marks={speedMarks}
            step={1}
          />
        </Form.Item>

        <div style={{ marginBottom: 16, fontSize: 16, fontWeight: 500, borderBottom: '1px solid #e8e8e8', paddingBottom: 8 }}>
          界面设置
        </div>

        <Form.Item
          label="启用系统通知"
          name="enable_notification"
          valuePropName="checked"
          extra="启用后在文件传输完成时显示系统通知"
        >
          <Switch />
        </Form.Item>

        <Form.Item
          label="主题模式"
          name="theme"
          extra="选择应用界面主题，跟随系统将自动适配系统深色模式"
        >
          <Select style={{ width: 200 }}>
            <Option value="auto">
              <Space>
                <span>🖥️</span>
                <span>跟随系统</span>
              </Space>
            </Option>
            <Option value="light">
              <Space>
                <span>☀️</span>
                <span>浅色模式</span>
              </Space>
            </Option>
            <Option value="dark">
              <Space>
                <span>🌙</span>
                <span>深色模式</span>
              </Space>
            </Option>
          </Select>
        </Form.Item>

        <Divider />

        <Form.Item>
          <Space>
            <Button
              type="primary"
              htmlType="submit"
              icon={<SaveOutlined />}
              loading={isSaving}
              size="large"
            >
              保存设置
            </Button>
            <Button htmlType="reset" onClick={loadSettings} size="large">
              重置
            </Button>
          </Space>
        </Form.Item>
      </Form>

      <Divider />

      <div style={{ color: '#999', fontSize: 12 }}>
        <Row gutter={16}>
          <Col span={12}>
            <p><strong>温馨提示：</strong></p>
            <ul style={{ margin: 0, paddingLeft: 20 }}>
              <li>设备名称修改后需要重启应用生效</li>
              <li>下载路径可以随时修改，不影响已进行的传输</li>
              <li>速度限制仅对新的传输任务生效</li>
              <li>Windows 和 macOS 的开机自启机制不同，请确保应用安装在系统盘</li>
            </ul>
          </Col>
          <Col span={12}>
            <p><strong>快捷键：</strong></p>
            <ul style={{ margin: 0, paddingLeft: 20 }}>
              <li>Ctrl/Cmd + V：粘贴文件路径</li>
              <li>Ctrl/Cmd + R：刷新设备列表</li>
              <li>Ctrl/Cmd + ,：打开设置</li>
            </ul>
          </Col>
        </Row>
      </div>
    </Card>
  )
}
