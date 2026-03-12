import { useState } from 'react'
import { Card } from 'antd'
import DeviceList from '../components/device/DeviceList'
import TransferList from '../components/transfer/TransferList'
import FileDropZone from '../components/transfer/FileDropZone'

export default function HomePage() {
  const [selectedDeviceId, setSelectedDeviceId] = useState<string | null>(null)

  return (
    <div style={{ display: 'flex', gap: '24px', height: '100%' }}>
      <Card
        title="在线设备"
        style={{ width: 350, flexShrink: 0 }}
        bodyStyle={{ height: 'calc(100% - 57px)', overflow: 'auto' }}
      >
        <DeviceList
          selectedDeviceId={selectedDeviceId}
          onSelectDevice={setSelectedDeviceId}
        />
      </Card>

      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: '24px' }}>
        <FileDropZone selectedDeviceId={selectedDeviceId} />

        <Card
          title="传输列表"
          style={{ flex: 1 }}
          bodyStyle={{ height: 'calc(100% - 57px)', overflow: 'auto' }}
        >
          <TransferList />
        </Card>
      </div>
    </div>
  )
}
