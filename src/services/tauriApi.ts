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

// Device connection status for UI
export type DeviceStatus = 'scanning' | 'connecting' | 'online' | 'offline'

// Device status colors (from design review)
export const DEVICE_STATUS_COLORS: Record<DeviceStatus, string> = {
  scanning: '#9CA3AF',   // 灰色 - 正在扫描
  connecting: '#F59E0B', // 黄色 - 连接中
  online: '#10B981',     // 绿色 - 在线
  offline: '#6B7280',    // 深灰色 - 离线
}

// Device status labels
export const DEVICE_STATUS_LABELS: Record<DeviceStatus, string> = {
  scanning: '扫描中',
  connecting: '连接中',
  online: '在线',
  offline: '离线',
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
  file_paths: string[]
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

// 打开已传输成功的文件 / 所在文件夹
export async function openTransferFile(taskId: string, index: number): Promise<void> {
  return invoke('open_transfer_file', { taskId, index })
}

export async function revealTransferFile(taskId: string, index: number): Promise<void> {
  return invoke('reveal_transfer_file', { taskId, index })
}

// 后端限速单位为字节/秒，前端展示与输入使用 MB/s
const BYTES_PER_MB = 1024 * 1024

export async function getSettings(): Promise<Settings> {
  const settings = await invoke<Settings>('get_settings')
  return {
    ...settings,
    upload_limit: Math.round(settings.upload_limit / BYTES_PER_MB),
    download_limit: Math.round(settings.download_limit / BYTES_PER_MB),
  }
}

export async function saveSettings(settings: Settings): Promise<void> {
  return invoke('save_settings', {
    settings: {
      ...settings,
      upload_limit: settings.upload_limit * BYTES_PER_MB,
      download_limit: settings.download_limit * BYTES_PER_MB,
    },
  })
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

// 本机设备信息
export interface LocalDeviceInfo {
  device_id: string
  device_name: string
}

export async function getLocalDeviceInfo(): Promise<LocalDeviceInfo> {
  return invoke('get_local_device_info')
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

// 下载目录权限检测
export interface DownloadPathStatus {
  path: string
  writable: boolean
  message: string | null
  suggested_path: string | null
}

export async function checkDownloadPath(path?: string): Promise<DownloadPathStatus> {
  return invoke('check_download_path', { path: path ?? null })
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
