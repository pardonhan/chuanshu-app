import { create } from 'zustand'
import type { DeviceInfo } from '../services/tauriApi'

interface DeviceState {
  devices: DeviceInfo[]
  selectedDeviceId: string | null
  isLoading: boolean
  error: string | null

  // Actions
  setDevices: (devices: DeviceInfo[]) => void
  addDevice: (device: DeviceInfo) => void
  removeDevice: (deviceId: string) => void
  updateDevice: (device: DeviceInfo) => void
  selectDevice: (deviceId: string | null) => void
  setLoading: (loading: boolean) => void
  setError: (error: string | null) => void
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
}))
