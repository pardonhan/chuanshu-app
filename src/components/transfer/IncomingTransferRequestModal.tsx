import { Modal, Descriptions, List, Typography, message } from 'antd'
import { useEffect, useState } from 'react'
import {
  acceptTransferRequest,
  onTransferRequest,
  rejectTransferRequest,
  type IncomingTransferRequest,
} from '../../services/tauriApi'
import { filesize } from 'filesize'

export default function IncomingTransferRequestModal() {
  const [request, setRequest] = useState<IncomingTransferRequest | null>(null)
  const [loading, setLoading] = useState(false)

  useEffect(() => {
    let unlisten: (() => void) | undefined
    onTransferRequest((incoming) => setRequest(incoming)).then((cleanup) => {
      unlisten = cleanup
    })
    return () => unlisten?.()
  }, [])

  const resolve = async (accepted: boolean) => {
    if (!request) return
    setLoading(true)
    try {
      if (accepted) {
        await acceptTransferRequest(request.task_id)
      } else {
        await rejectTransferRequest(request.task_id)
      }
      setRequest(null)
    } catch (error) {
      message.error(error instanceof Error ? error.message : '处理传输请求失败')
    } finally {
      setLoading(false)
    }
  }

  return (
    <Modal
      open={request !== null}
      title="收到文件传输请求"
      okText="接受"
      cancelText="拒绝"
      confirmLoading={loading}
      onOk={() => resolve(true)}
      onCancel={() => resolve(false)}
      maskClosable={false}
      destroyOnClose
    >
      {request && (
        <>
          <Descriptions column={1} size="small" bordered>
            <Descriptions.Item label="发送设备">{request.sender_name}</Descriptions.Item>
            <Descriptions.Item label="文件数量">{request.file_count}</Descriptions.Item>
            <Descriptions.Item label="总大小">{filesize(request.total_size, { base: 2 })}</Descriptions.Item>
          </Descriptions>
          <Typography.Text strong style={{ display: 'block', marginTop: 16 }}>文件</Typography.Text>
          <List
            size="small"
            dataSource={request.files.slice(0, 8)}
            renderItem={(file) => (
              <List.Item>
                <span>{file.relative_path ? `${file.relative_path}/${file.file_name}` : file.file_name}</span>
                <Typography.Text type="secondary">{filesize(file.file_size, { base: 2 })}</Typography.Text>
              </List.Item>
            )}
          />
          {request.files.length > 8 && (
            <Typography.Text type="secondary">还有 {request.files.length - 8} 个文件</Typography.Text>
          )}
        </>
      )}
    </Modal>
  )
}
