import { useState } from 'react'
import { Upload, Card, message, Modal, List, Button } from 'antd'
import { InboxOutlined, SendOutlined } from '@ant-design/icons'
import type { UploadProps, UploadFile } from 'antd'
import { useDeviceStore } from '../../store/useDeviceStore'
import { sendFiles } from '../../services/tauriApi'

const { Dragger } = Upload

interface FileDropZoneProps {
  selectedDeviceId: string | null
}

export default function FileDropZone({ selectedDeviceId }: FileDropZoneProps) {
  const { devices } = useDeviceStore()
  const [fileList, setFileList] = useState<UploadFile[]>([])
  const [isModalOpen, setIsModalOpen] = useState(false)
  const [isSending, setIsSending] = useState(false)

  const props: UploadProps = {
    name: 'file',
    multiple: true,
    fileList,
    beforeUpload: () => {
      if (!selectedDeviceId) {
        message.error('请先选择要发送的设备')
        return false
      }
      return false // Prevent auto upload
    },
    onChange: (info) => {
      setFileList(info.fileList)
    },
    onDrop: () => {
      if (!selectedDeviceId) {
        message.error('请先选择要发送的设备')
        return
      }
    },
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
      const filePaths = fileList
        .map((f) => (f.originFileObj as any)?.path)
        .filter((path): path is string => !!path)

      if (filePaths.length === 0) {
        // Fallback: try to use the file name if path is not available
        message.warning('无法获取文件路径，请使用文件选择对话框')
        return
      }

      await sendFiles([selectedDeviceId], filePaths)
      message.success('文件发送请求已发送')
      setFileList([])
      setIsModalOpen(false)
    } catch (error) {
      console.error('Failed to send files:', error)
      message.error('发送失败: ' + String(error))
    } finally {
      setIsSending(false)
    }
  }

  const selectedDevice = devices.find(d => d.device_id === selectedDeviceId)

  return (
    <>
      <Card>
        <Dragger {...props} disabled={!selectedDeviceId}>
          <p className="ant-upload-drag-icon">
            <InboxOutlined />
          </p>
          <p className="ant-upload-text">点击或拖拽文件/文件夹到此处</p>
          <p className="ant-upload-hint">
            {selectedDeviceId
              ? `将发送到: ${selectedDevice?.device_name || '未知设备'}`
              : '请先在左侧选择目标设备'}
          </p>
        </Dragger>

        {fileList.length > 0 && (
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
