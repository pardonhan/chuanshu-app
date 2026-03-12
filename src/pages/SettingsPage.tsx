import { useEffect, useState } from 'react'
import { Card, Form, Input, Switch, Select, Button, Space, Divider, message } from 'antd'
import { SaveOutlined } from '@ant-design/icons'
import { getSettings, saveSettings, type Settings } from '../services/tauriApi'

const { Option } = Select

export default function SettingsPage() {
  const [form] = Form.useForm()
  const [isLoading, setIsLoading] = useState(false)
  const [isSaving, setIsSaving] = useState(false)

  useEffect(() => {
    loadSettings()
  }, [])

  const loadSettings = async () => {
    setIsLoading(true)
    try {
      const settings = await getSettings()
      form.setFieldsValue(settings)
    } catch (error) {
      console.error('Failed to load settings:', error)
      message.error('加载设置失败')
    } finally {
      setIsLoading(false)
    }
  }

  const onFinish = async (values: Settings) => {
    setIsSaving(true)
    try {
      await saveSettings(values)
      message.success('设置已保存')
    } catch (error) {
      console.error('Failed to save settings:', error)
      message.error('保存设置失败')
    } finally {
      setIsSaving(false)
    }
  }

  return (
    <Card title="设置" loading={isLoading}>
      <Form
        form={form}
        layout="vertical"
        onFinish={onFinish}
        style={{ maxWidth: 600 }}
      >
        <Form.Item
          label="设备名称"
          name="device_name"
          rules={[{ required: true, message: '请输入设备名称' }]}
        >
          <Input placeholder="输入设备在局域网中显示的名称" />
        </Form.Item>

        <Form.Item
          label="默认下载路径"
          name="download_path"
          rules={[{ required: true, message: '请输入下载路径' }]}
        >
          <Input placeholder="文件接收后的保存路径" />
        </Form.Item>

        <Divider>传输设置</Divider>

        <Form.Item
          label="自动接收文件"
          name="auto_accept"
          valuePropName="checked"
        >
          <Switch />
        </Form.Item>

        <Form.Item
          label="上传限速 (MB/s)"
          name="upload_limit"
        >
          <Select style={{ width: 200 }}>
            <Option value={0}>不限速</Option>
            <Option value={1}>1 MB/s</Option>
            <Option value={5}>5 MB/s</Option>
            <Option value={10}>10 MB/s</Option>
            <Option value={50}>50 MB/s</Option>
            <Option value={100}>100 MB/s</Option>
          </Select>
        </Form.Item>

        <Form.Item
          label="下载限速 (MB/s)"
          name="download_limit"
        >
          <Select style={{ width: 200 }}>
            <Option value={0}>不限速</Option>
            <Option value={1}>1 MB/s</Option>
            <Option value={5}>5 MB/s</Option>
            <Option value={10}>10 MB/s</Option>
            <Option value={50}>50 MB/s</Option>
            <Option value={100}>100 MB/s</Option>
          </Select>
        </Form.Item>

        <Divider>界面设置</Divider>

        <Form.Item
          label="启用系统通知"
          name="enable_notification"
          valuePropName="checked"
        >
          <Switch />
        </Form.Item>

        <Form.Item
          label="主题"
          name="theme"
        >
          <Select style={{ width: 200 }}>
            <Option value="auto">跟随系统</Option>
            <Option value="light">浅色模式</Option>
            <Option value="dark">深色模式</Option>
          </Select>
        </Form.Item>

        <Form.Item>
          <Space>
            <Button type="primary" htmlType="submit" icon={<SaveOutlined />} loading={isSaving}>
              保存设置
            </Button>
            <Button htmlType="reset" onClick={loadSettings}>重置</Button>
          </Space>
        </Form.Item>
      </Form>
    </Card>
  )
}
