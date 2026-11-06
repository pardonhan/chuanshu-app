import { create } from 'zustand'
import type { TransferTaskInfo } from '../services/tauriApi'

interface TransferState {
  tasks: TransferTaskInfo[]
  isLoading: boolean
  error: string | null

  // Actions
  setTasks: (tasks: TransferTaskInfo[]) => void
  updateTaskProgress: (taskId: string, progress: number, speed: number, transferredSize: number) => void
  setTaskStatus: (taskId: string, status: TransferTaskInfo['status']) => void
  setLoading: (loading: boolean) => void
  setError: (error: string | null) => void
}

export const useTransferStore = create<TransferState>((set, get) => ({
  tasks: [],
  isLoading: false,
  error: null,

  setTasks: (tasks) => set({ tasks }),

  updateTaskProgress: (taskId, progress, speed, transferredSize) => {
    const { tasks } = get()
    set({
      tasks: tasks.map(t =>
        t.task_id === taskId
          ? { ...t, progress, speed, transferred_size: transferredSize }
          : t
      )
    })
  },

  setTaskStatus: (taskId, status) => {
    const { tasks } = get()
    set({
      tasks: tasks.map(t =>
        t.task_id === taskId ? { ...t, status } : t
      )
    })
  },

  setLoading: (loading) => set({ isLoading: loading }),

  setError: (error) => set({ error }),
}))
