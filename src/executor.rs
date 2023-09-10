use crate::waker::Signal;
use futures::future::BoxFuture;
use std::{
    cell::RefCell,
    collections::VecDeque,
    future::Future,
    sync::mpsc,
    sync::{Arc, Mutex},
    task::{Context, Poll, Wake, Waker},
    thread::JoinHandle,
};

scoped_tls::scoped_thread_local!(pub(crate) static EX: Executor);

// 执行器结构体
pub struct Executor {
    local_queue: TaskQueue,  // 本地任务队列
    thread_pool: ThreadPool, // 线程池
}

impl Executor {
    // 创建新的执行器实例
    pub fn new() -> Executor {
        Executor {
            local_queue: TaskQueue::new(),
            thread_pool: ThreadPool::new(2), // 线程池大小为2
        }
    }

    // 添加任务到执行器中
    pub fn spawn(fut: impl Future<Output = ()> + 'static + Send) {
        let t = Arc::new(Task {
            future: RefCell::new(Box::pin(fut)), // 将任务包装成BoxFuture，并存储在Task中
            signal: Arc::new(Signal::new()),     // 为任务创建一个信号量
        });
        EX.with(|ex| ex.local_queue.push(t.clone())); // 将任务添加到本地任务队列中
    }

    // 阻塞执行给定的future，直到完成，并返回结果
    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        let mut main_fut = std::pin::Pin::new(future);
        let signal: Arc<Signal> = Arc::new(Signal::new()); // 为主任务创建一个信号量
        let waker = Waker::from(signal.clone());

        let mut cx = Context::from_waker(&waker);

        EX.set(self, || {
            loop {
                // 如果主任务已经完成，返回其输出
                if let Poll::Ready(t) = main_fut.as_mut().poll(&mut cx) {
                    break t;
                }

                // 处理本地任务队列中的任务
                while let Some(t) = self.local_queue.pop() {
                    let _ = self.thread_pool.execute(t);
                }

                // 如果没有任务可执行，等待信号
                signal.wait();
            }
        })
    }
}

// 任务结构体
pub struct Task {
    future: RefCell<BoxFuture<'static, ()>>, // 异步任务的future
    signal: Arc<Signal>,                     // 任务的信号量
}

unsafe impl Send for Task {}
unsafe impl Sync for Task {}

impl Wake for Task {
    // 当任务被唤醒时，将其添加到本地任务队列，并触发信号量通知
    fn wake(self: Arc<Self>) {
        RUNNABLE.with(|runnable| runnable.lock().unwrap().push_back(self.clone()));
        self.signal.notify();
    }
}

// 本地任务队列
pub struct TaskQueue {
    queue: RefCell<VecDeque<Arc<Task>>>,
}

impl TaskQueue {
    // 创建新的任务队列实例
    pub fn new() -> Self {
        const DEFAULT_TASK_QUEUE_SIZE: usize = 4096;
        Self::new_with_capacity(DEFAULT_TASK_QUEUE_SIZE)
    }

    // 创建具有指定容量的任务队列实例
    pub fn new_with_capacity(capacity: usize) -> Self {
        Self {
            queue: RefCell::new(VecDeque::with_capacity(capacity)),
        }
    }

    // 将任务添加到队列中
    pub(crate) fn push(&self, runnable: Arc<Task>) {
        self.queue.borrow_mut().push_back(runnable);
    }

    // 从队列中取出任务
    pub(crate) fn pop(&self) -> Option<Arc<Task>> {
        self.queue.borrow_mut().pop_front()
    }
}

// 工作线程结构体
struct Worker {
    wthread: Option<JoinHandle<()>>,
}

impl Worker {
    // 创建新的工作线程实例
    fn new(receiver: Arc<Mutex<mpsc::Receiver<Option<Arc<Task>>>>>) -> Self {
        let thread = std::thread::spawn(move || loop {
            let task = receiver.lock().unwrap().recv().unwrap();
            match task {
                Some(task) => {
                    let waker = Waker::from(task.clone());
                    let mut cx = Context::from_waker(&waker);
                    let _ = task.future.borrow_mut().as_mut().poll(&mut cx);
                }
                None => {
                    break;
                }
            }
        });
        Worker {
            wthread: Some(thread),
        }
    }
}

// 线程池结构体
struct ThreadPool {
    workers: Vec<Worker>,                    // 工作线程集合
    sender: mpsc::Sender<Option<Arc<Task>>>, // 用于向工作线程发送任务的通道
}

impl ThreadPool {
    // 创建新的线程池实例
    fn new(max_worker: usize) -> Self {
        if max_worker == 0 {
            panic!("max_worker must be greater than 0.")
        }

        let (sender, receiver) = mpsc::channel::<Option<Arc<Task>>>(); // 创建通道用于任务传输
        let receiver = Arc::new(Mutex::new(receiver));
        let mut workers = Vec::with_capacity(max_worker);

        for _ in 0..max_worker {
            workers.push(Worker::new(receiver.clone())); // 创建工作线程并将通道传递给它们
        }

        ThreadPool { workers, sender }
    }

    // 执行任务，并将其发送到线程池中的工作线程
    fn execute(&self, task: Arc<Task>) -> Poll<()> {
        self.sender.send(Some(task)).unwrap();
        Poll::Pending
    }
}

impl Drop for ThreadPool {
    // 在线程池被丢弃时，向工作线程发送终止信号，等待线程结束
    fn drop(&mut self) {
        for _i in 0..self.workers.len() {
            let _ = self.sender.send(None);
        }

        for worker in &mut self.workers {
            if let Some(thread) = worker.wthread.take() {
                let _ = thread.join();
            }
        }
    }
}
