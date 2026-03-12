import { useState, useEffect } from 'react'
import { Card, message, Modal, List, Button } from 'antd'
import { InboxOutlined, SendOutlined } from '@ant-design/icons'
import { useDeviceStore } from '../../store/useDeviceStore'
import { sendFiles } from '../../services/tauriApi'
import { listen } from '@tauri-apps/api/event'

interface FileItem {
  name: string
  path: string
  size?: number
}

interface FileDropZoneProps {
  selectedDeviceId: string | null
}

export default function FileDropZone({ selectedDeviceId }: FileDropZoneProps) {
  const { devices } = useDeviceStore()
  const [fileList, setFileList] = useState<FileItem[]>([])
  const [isModalOpen, setIsModalOpen] = useState(false)
  const [isSending, setIsSending] = useState(false)

  useEffect(() => {
    // Listen for Tauri file drop events
    const setupDragDrop = async () => {
      try {
        const unlisten = await listen<string[]>('tauri://file-drop', (event) => {
          handleFileDrop(event.payload)
        })

        return unlisten
      } catch (error) {
        console.error('Failed to setup drag-drop:', error)
      }
    }

    setupDragDrop()
  }, [selectedDeviceId])

  const handleFileDrop = (paths: string[]) => {
    if (!selectedDeviceId) {
      message.error('请先选择要发送的设备')
      return
    }

    const newFiles: FileItem[] = paths.map((path) => ({
      name: path.split('/').pop() || path.split('\\').pop() || path,
      path,
    }))

    setFileList((prev) => [...prev, ...newFiles])
    message.success(`已添加 ${newFiles.length} 个文件`)
  }

  const handleRemoveFile = (index: number) => {
    setFileList((prev) => prev.filter((_, i) => i !== index))
  }

  const handleSend = async () => {
    if (!selectedDeviceId) {
      message.error('请先选择要发送的设备')
      return
    }

    if (fileList.length === 0) {
      message.error('请先添加要发送的文件')
      return
    }

    const device = devices.find(d => d.device_id === selectedDeviceId)
    if (!device) {
      message.error('所选设备已离线')
      return
    }

    setIsModalOpen(true)
  }

  const confirmSend = async () => {
    if (!selectedDeviceId || fileList.length === 0) return

    setIsSending(true)
    try {
      const filePaths = fileList.map((f) => f.path)
      await sendFiles([selectedDeviceId], filePaths)
      message.success('文件发送请求已发送')
      setFileList([])
      setIsModalOpen(false)
    } catch (error) {
      console.error('Failed to send files:', error)
      message.error('发送失败：' + String(error))
    } finally {
      setIsSending(false)
    }
  }

  const selectedDevice = devices.find(d => d.device_id === selectedDeviceId)

  return (
    <>
      <Card>
        <div
          style={{
            border: '2px dashed #d9d9d9',
            borderRadius: '6px',
            padding: '20px',
            textAlign: 'center',
            backgroundColor: fileList.length > 0 ? '#fafafa' : '#fff',
            cursor: selectedDeviceId ? 'pointer' : 'not-allowed',
            opacity: selectedDeviceId ? 1 : 0.5,
          }}
        >
          <InboxOutlined style={{ fontSize: 48, color: selectedDeviceId ? '#1890ff' : '#ccc' }} />
          <p style={{ marginTop: 16, fontSize: 16 }}>
            拖拽文件/文件夹到此处
          </p>
          <p style={{ color: '#999', fontSize: 12 }}>
            {selectedDeviceId
              ? `将发送到：${selectedDevice?.device_name || '未知设备'}`
              : '请先在左侧选择目标设备'}
          </p>
        </div>

        {fileList.length > 0 && (
          <div style={{ marginTop: 16 }}>
            <List
              size="small"
              dataSource={fileList}
              renderItem={(item, index) => (
                <List.Item
                  actions={[
                    <Button
                      type="text"
                      danger
                      size="small"
                      onClick={() => handleRemoveFile(index)}
                    >
                      删除
                    </Button>,
                  ]}
                >
                  {item.name}
                </List.Item>
              )}
            />
            <div style={{ marginTop: 16, textAlign: 'right' }}>
              <Button
                type="primary"
                icon={<SendOutlined />}
                onClick={handleSend}
                disabled={!selectedDeviceId}
              >
                发送 {fileList.length} 个文件
              </Button>
            </div>
          </div>
        )}
      </Card>

      <Modal
        title="确认发送"
        open={isModalOpen}
        onOk={confirmSend}
        onCancel={() => setIsModalOpen(false)}
        confirmLoading={isSending}
        okText="确认发送"
        cancelText="取消"
      >
        <p>您即将发送以下文件到 <strong>{selectedDevice?.device_name}</strong>:</p>
        <List
          size="small"
          dataSource={fileList.slice(0, 5)}
          renderItem={item => (
            <List.Item>{item.name}</List.Item>
          )}
          footer={
            fileList.length > 5
              ? `...还有 ${fileList.length - 5} 个文件`
              : undefined
          }
        />
      </Modal>
    </>
  )
}
