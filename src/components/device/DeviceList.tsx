import { useEffect, useCallback, useState } from 'react'
import { List, Avatar, Badge, Button, Space, Empty, Input, message, Tag } from 'antd'
import { DesktopOutlined, ReloadOutlined, PlusOutlined } from '@ant-design/icons'
import { useDeviceStore } from '../../store/useDeviceStore'
import { getKnownDevices, onDeviceOnline, onDeviceOffline, discoverDeviceByIp, type DeviceInfo } from '../../services/tauriApi'
import type { UnlistenFn } from '@tauri-apps/api/event'

interface DeviceListProps {
  selectedDeviceId: string | null
  onSelectDevice: (deviceId: string) => void
}

const getOsIcon = (os: string) => {
  if (os === 'Windows') return <DesktopOutlined style={{ color: '#0078d4' }} />
  if (os === 'MacOS') return <DesktopOutlined style={{ color: '#000000' }} />
  return <DesktopOutlined />
}

const getOsLabel = (os: string) => {
  switch (os) {
    case 'Windows': return 'Windows'
    case 'MacOS': return 'macOS'
    case 'Linux': return 'Linux'
    default: return '未知系统'
  }
}

export default function DeviceList({ selectedDeviceId, onSelectDevice }: DeviceListProps) {
  const { devices, isLoading, loadKnownDevices, setDeviceOnline, setDeviceOffline, setLoading } = useDeviceStore()
  const [inputIp, setInputIp] = useState('')
  const [isDiscovering, setIsDiscovering] = useState(false)

  // Load known devices on mount (includes offline devices)
  const loadDevices = useCallback(async () => {
    setLoading(true)
    try {
      const knownDevices = await getKnownDevices()
      loadKnownDevices(knownDevices)
    } catch (error) {
      console.error('Failed to load known devices:', error)
    } finally {
      setLoading(false)
    }
  }, [loadKnownDevices, setLoading])

  // Discover device by IP
  const handleDiscoverByIp = async () => {
    if (!inputIp.trim()) {
      message.warning('请输入 IP 地址')
      return
    }

    setIsDiscovering(true)
    try {
      const device = await discoverDeviceByIp(inputIp.trim())
      if (device) {
        message.success(`发现设备：${device.device_name}`)
      } else {
        message.warning('未发现设备，请检查 IP 地址是否正确')
      }
    } catch (error) {
      console.error('Failed to discover device:', error)
      message.error('发现设备失败')
    } finally {
      setIsDiscovering(false)
    }
  }

  useEffect(() => {
    loadDevices()

    // Set up event listeners
    let unlistenOnline: UnlistenFn | null = null
    let unlistenOffline: UnlistenFn | null = null

    const setupListeners = async () => {
      unlistenOnline = await onDeviceOnline((device: DeviceInfo) => {
        setDeviceOnline(device)
      })

      unlistenOffline = await onDeviceOffline((deviceId: string) => {
        setDeviceOffline(deviceId)
      })
    }

    setupListeners()

    // Poll for updates every 5 seconds as fallback
    const interval = setInterval(loadDevices, 5000)

    return () => {
      clearInterval(interval)
      unlistenOnline?.()
      unlistenOffline?.()
    }
  }, [loadDevices, setDeviceOnline, setDeviceOffline])

  return (
    <div>
      <div style={{ marginBottom: 16, display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
        <span style={{ fontSize: 12, color: '#666' }}>
          {devices.length > 0 ? `${devices.length} 个设备在线` : '未发现设备'}
        </span>
        <Button
          type="text"
          icon={<ReloadOutlined />}
          size="small"
          loading={isLoading}
          onClick={loadDevices}
        >
          刷新
        </Button>
      </div>

      {/* IP 输入框 */}
      <Space.Compact style={{ width: '100%', marginBottom: 16 }}>
        <Input
          placeholder="输入 IP 地址发现设备"
          value={inputIp}
          onChange={(e) => setInputIp(e.target.value)}
          onPressEnter={handleDiscoverByIp}
          disabled={isDiscovering}
        />
        <Button
          type="primary"
          icon={<PlusOutlined />}
          loading={isDiscovering}
          onClick={handleDiscoverByIp}
        >
          发现
        </Button>
      </Space.Compact>

      {devices.length === 0 ? (
        <Empty
          image={Empty.PRESENTED_IMAGE_SIMPLE}
          description="暂无设备记录"
          style={{ marginTop: 40 }}
        />
      ) : (
        <List
          dataSource={devices}
          loading={isLoading}
          renderItem={(device) => (
            <List.Item
              key={device.device_id}
              style={{
                cursor: 'pointer',
                borderRadius: 8,
                padding: 12,
                backgroundColor: selectedDeviceId === device.device_id ? '#e6f7ff' : 'transparent',
                border: selectedDeviceId === device.device_id ? '1px solid #91d5ff' : '1px solid transparent',
                transition: 'all 0.2s',
                opacity: device.is_online ? 1 : 0.5,
              }}
              onClick={() => device.is_online && onSelectDevice(device.device_id)}
            >
              <List.Item.Meta
                avatar={
                  <Badge status={device.is_online ? 'success' : 'default'}>
                    <Avatar icon={getOsIcon(device.os)} />
                  </Badge>
                }
                title={
                  <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                    <span style={{ fontWeight: 500, color: device.is_online ? undefined : '#999' }}>
                      {device.device_name}
                    </span>
                    {device.is_online ? (
                      <Badge status="success" text="在线" style={{ fontSize: 12 }} />
                    ) : (
                      <Tag color="default" style={{ fontSize: 12 }}>离线</Tag>
                    )}
                  </div>
                }
                description={
                  <Space direction="vertical" size={0} style={{ fontSize: 12 }}>
                    <span>{device.ip_address}</span>
                    <span style={{ color: '#999' }}>{getOsLabel(device.os)}</span>
                    {!device.is_online && device.last_connected && (
                      <span style={{ color: '#bbb' }}>
                        最后连接：{new Date(device.last_connected * 1000).toLocaleString('zh-CN')}
                      </span>
                    )}
                  </Space>
                }
              />
            </List.Item>
          )}
        />
      )}
    </div>
  )
}
