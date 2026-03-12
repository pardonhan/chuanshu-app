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
