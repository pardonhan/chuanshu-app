import { create } from 'zustand'
import type { DeviceInfo, KnownDevice, DeviceStatus } from '../services/tauriApi'

export interface DeviceWithStatus extends DeviceInfo {
  is_online: boolean
  status: DeviceStatus  // 四态：scanning | connecting | online | offline
  last_connected?: number | null
}

interface DeviceState {
  devices: DeviceWithStatus[]
  selectedDeviceId: string | null
  isLoading: boolean
  error: string | null

  // Actions
  setDevices: (devices: DeviceWithStatus[]) => void
  addDevice: (device: DeviceWithStatus) => void
  removeDevice: (deviceId: string) => void
  updateDevice: (device: DeviceWithStatus) => void
  selectDevice: (deviceId: string | null) => void
  setLoading: (loading: boolean) => void
  setError: (error: string | null) => void
  // Load known devices and merge with online status
  loadKnownDevices: (knownDevices: KnownDevice[]) => void
  // Update device online status from event
  setDeviceOnline: (device: DeviceInfo) => void
  setDeviceOffline: (deviceId: string) => void
  setDeviceStatus: (deviceId: string, status: DeviceStatus) => void
}

export const useDeviceStore = create<DeviceState>((set, get) => ({
  devices: [],
  selectedDeviceId: null,
  isLoading: false,
  error: null,

  setDevices: (devices) => set({ devices }),

  addDevice: (device) => {
    const { devices } = get()
    const exists = devices.find(d => d.device_id === device.device_id)
    if (exists) {
      // Update existing device
      set({
        devices: devices.map(d =>
          d.device_id === device.device_id ? device : d
        )
      })
    } else {
      // Add new device
      set({ devices: [...devices, device] })
    }
  },

  removeDevice: (deviceId) => {
    const { devices, selectedDeviceId } = get()
    set({
      devices: devices.filter(d => d.device_id !== deviceId),
      selectedDeviceId: selectedDeviceId === deviceId ? null : selectedDeviceId
    })
  },

  updateDevice: (device) => {
    const { devices } = get()
    set({
      devices: devices.map(d =>
        d.device_id === device.device_id ? device : d
      )
    })
  },

  selectDevice: (deviceId) => set({ selectedDeviceId: deviceId }),

  setLoading: (loading) => set({ isLoading: loading }),

  setError: (error) => set({ error }),

  // Load known devices and merge with online status
  loadKnownDevices: (knownDevices) => {
    const devices: DeviceWithStatus[] = knownDevices.map(kd => ({
      device_id: kd.device_id,
      device_name: kd.device_name,
      os: kd.os as 'Windows' | 'MacOS' | 'Linux' | 'Unknown',
      ip_address: kd.ip_address,
      quic_port: kd.quic_port,
      protocol_version: kd.protocol_version,
      capabilities: (() => {
        try {
          return JSON.parse(kd.capabilities)
        } catch {
          return []
        }
      })(),
      last_seen: kd.last_seen,
      is_online: kd.is_online,
      status: kd.is_online ? 'online' : 'offline',  // 从已知设备加载时，在线=online，否则=offline
      last_connected: kd.last_connected,
    }))

    // Sort by last_connected descending (most recently connected first)
    devices.sort((a, b) => {
      // Online devices first
      if (a.is_online && !b.is_online) return -1
      if (!a.is_online && b.is_online) return 1

      // Then by last_connected
      const aTime = a.last_connected || a.last_seen
      const bTime = b.last_connected || b.last_seen
      return bTime - aTime
    })

    set({ devices })
  },

  // Update device online status from event
  setDeviceOnline: (device) => {
    const { devices } = get()
    const exists = devices.find(d => d.device_id === device.device_id)

    if (exists) {
      // Update existing device to online
      set({
        devices: devices.map(d =>
          d.device_id === device.device_id
            ? { ...d, ...device, is_online: true, status: 'online' }
            : d
        )
      })
    } else {
      // Add new online device
      const newDevice: DeviceWithStatus = {
        ...device,
        is_online: true,
        status: 'online',
        last_connected: Date.now() / 1000,
      }
      set({ devices: [...devices, newDevice] })
    }
  },

  setDeviceOffline: (deviceId) => {
    const { devices } = get()
    set({
      devices: devices.map(d =>
        d.device_id === deviceId ? { ...d, is_online: false, status: 'offline' } : d
      )
    })
  },

  // Set device status (for scanning/connecting states)
  setDeviceStatus: (deviceId, status) => {
    const { devices } = get()
    set({
      devices: devices.map(d =>
        d.device_id === deviceId ? { ...d, status } : d
      )
    })
  },
}))
