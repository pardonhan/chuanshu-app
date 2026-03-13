import { invoke } from '@tauri-apps/api/core'
import { listen, type Event, type UnlistenFn } from '@tauri-apps/api/event'

// Device types
export interface DeviceInfo {
  device_id: string
  device_name: string
  os: 'Windows' | 'MacOS' | 'Linux' | 'Unknown'
  ip_address: string
  quic_port: number
  protocol_version: string
  capabilities: string[]
  last_seen: number
  is_online?: boolean  // Optional: for known devices that may be offline
}

// Known device type (includes offline devices)
export interface KnownDevice {
  device_id: string
  device_name: string
  os: string
  ip_address: string
  quic_port: number
  protocol_version: string
  capabilities: string  // JSON array string
  last_seen: number
  last_connected: number | null
  is_online: boolean
  created_at: number
}

// Transfer types
export type TransferStatus = 'Pending' | 'Transferring' | 'Paused' | 'Completed' | 'Failed' | 'Canceled'
export type TransferType = 'Send' | 'Receive'

export interface TransferTaskInfo {
  task_id: string
  peer_device_name: string
  transfer_type: TransferType
  status: TransferStatus
  total_size: number
  transferred_size: number
  file_count: number
  current_file: string
  speed: number
  progress: number
  error_message?: string
  created_at: number
}

// Settings type
export interface Settings {
  device_name: string
  download_path: string
  auto_accept: boolean
  upload_limit: number
  download_limit: number
  enable_notification: boolean
  theme: 'auto' | 'light' | 'dark'
}

// Transfer history type
export interface TransferHistoryEntry {
  task_id: string
  peer_device_name: string
  transfer_type: 'send' | 'receive'
  status: 'completed' | 'failed' | 'canceled' | 'pending' | 'transferring' | 'paused'
  total_size: number
  file_count: number
  file_names: string | null
  created_at: number
  completed_at: number | null
}

// Transfer history query type
export interface TransferHistoryQuery {
  limit: number
  offset: number
}

// IPC API functions
export async function getDeviceList(): Promise<DeviceInfo[]> {
  return invoke('get_device_list')
}

export async function discoverDeviceByIp(ip: string): Promise<DeviceInfo | null> {
  return invoke('discover_device_by_ip', { ip })
}

export async function getTransferTasks(): Promise<TransferTaskInfo[]> {
  return invoke('get_transfer_tasks')
}

export async function sendFiles(deviceIds: string[], filePaths: string[]): Promise<string[]> {
  return invoke('send_files', { request: { device_ids: deviceIds, file_paths: filePaths } })
}

export async function cancelTransfer(taskId: string): Promise<void> {
  return invoke('cancel_transfer', { taskId })
}

export async function pauseTransfer(taskId: string): Promise<void> {
  return invoke('pause_transfer', { taskId })
}

export async function resumeTransfer(taskId: string): Promise<void> {
  return invoke('resume_transfer', { taskId })
}

export async function getSettings(): Promise<Settings> {
  return invoke('get_settings')
}

export async function saveSettings(settings: Settings): Promise<void> {
  return invoke('save_settings', { settings })
}

export async function getTransferHistory(query: TransferHistoryQuery): Promise<TransferHistoryEntry[]> {
  return invoke('get_transfer_history', { query })
}

export async function clearTransferHistory(): Promise<void> {
  return invoke('clear_transfer_history')
}

// Known devices API
export async function getKnownDevices(): Promise<KnownDevice[]> {
  return invoke('get_known_devices')
}

export async function deleteKnownDevice(deviceId: string): Promise<void> {
  return invoke('delete_known_device', { deviceId })
}

// System info API
export interface SystemInfo {
  os: string
  os_version: string
  arch: string
  default_download_path: string
}

export async function getSystemInfo(): Promise<SystemInfo> {
  return invoke('get_system_info')
}

export async function setAutoLaunch(enabled: boolean): Promise<void> {
  return invoke('set_auto_launch', { enabled })
}

export async function getAutoLaunch(): Promise<boolean> {
  return invoke('get_auto_launch')
}

// Event listeners
export function onDeviceOnline(callback: (device: DeviceInfo) => void): Promise<UnlistenFn> {
  return listen('device-online', (event: Event<DeviceInfo>) => {
    callback(event.payload)
  })
}

export function onDeviceOffline(callback: (deviceId: string) => void): Promise<UnlistenFn> {
  return listen('device-offline', (event: Event<string>) => {
    callback(event.payload)
  })
}

export function onTransferProgress(callback: (task: TransferTaskInfo) => void): Promise<UnlistenFn> {
  return listen('transfer-progress', (event: Event<TransferTaskInfo>) => {
    callback(event.payload)
  })
}

export function onTransferCompleted(callback: (taskId: string) => void): Promise<UnlistenFn> {
  return listen('transfer-completed', (event: Event<string>) => {
    callback(event.payload)
  })
}

export function onTransferFailed(callback: (payload: { taskId: string; error: string }) => void): Promise<UnlistenFn> {
  return listen('transfer-failed', (event: Event<{ taskId: string; error: string }>) => {
    callback(event.payload)
  })
}
