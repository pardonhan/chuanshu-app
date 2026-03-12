import { create } from 'zustand'
import type { TransferTaskInfo } from '../services/tauriApi'

interface TransferState {
  tasks: TransferTaskInfo[]
  isLoading: boolean
  error: string | null

  // Actions
  setTasks: (tasks: TransferTaskInfo[]) => void
  addTask: (task: TransferTaskInfo) => void
  updateTask: (task: TransferTaskInfo) => void
  removeTask: (taskId: string) => void
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

  addTask: (task) => {
    const { tasks } = get()
    const exists = tasks.find(t => t.task_id === task.task_id)
    if (!exists) {
      set({ tasks: [task, ...tasks] })
    }
  },

  updateTask: (task) => {
    const { tasks } = get()
    set({
      tasks: tasks.map(t =>
        t.task_id === task.task_id ? task : t
      )
    })
  },

  removeTask: (taskId) => {
    const { tasks } = get()
    set({ tasks: tasks.filter(t => t.task_id !== taskId) })
  },

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
