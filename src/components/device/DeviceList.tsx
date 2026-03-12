import { useEffect, useCallback } from 'react'
import { List, Avatar, Badge, Button, Space, Empty } from 'antd'
import { DesktopOutlined, ReloadOutlined } from '@ant-design/icons'
import { useDeviceStore } from '../../store/useDeviceStore'
import { getDeviceList, onDeviceOnline, onDeviceOffline, type DeviceInfo } from '../../services/tauriApi'
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
  const { devices, isLoading, setDevices, addDevice, removeDevice, setLoading } = useDeviceStore()

  // Load devices on mount
  const loadDevices = useCallback(async () => {
    setLoading(true)
    try {
      const deviceList = await getDeviceList()
      setDevices(deviceList)
    } catch (error) {
      console.error('Failed to load devices:', error)
    } finally {
      setLoading(false)
    }
  }, [setDevices, setLoading])

  useEffect(() => {
    loadDevices()

    // Set up event listeners
    let unlistenOnline: UnlistenFn | null = null
    let unlistenOffline: UnlistenFn | null = null

    const setupListeners = async () => {
      unlistenOnline = await onDeviceOnline((device: DeviceInfo) => {
        addDevice(device)
      })

      unlistenOffline = await onDeviceOffline((deviceId: string) => {
        removeDevice(deviceId)
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
  }, [loadDevices, addDevice, removeDevice])

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

      {devices.length === 0 ? (
        <Empty
          image={Empty.PRESENTED_IMAGE_SIMPLE}
          description="暂无在线设备"
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
              }}
              onClick={() => onSelectDevice(device.device_id)}
            >
              <List.Item.Meta
                avatar={
                  <Badge status="success">
                    <Avatar icon={getOsIcon(device.os)} />
                  </Badge>
                }
                title={
                  <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
                    <span style={{ fontWeight: 500 }}>{device.device_name}</span>
                    <Badge status="success" text="在线" style={{ fontSize: 12 }} />
                  </div>
                }
                description={
                  <Space direction="vertical" size={0} style={{ fontSize: 12 }}>
                    <span>{device.ip_address}</span>
                    <span style={{ color: '#999' }}>{getOsLabel(device.os)}</span>
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
