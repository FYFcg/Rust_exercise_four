use std::task::{RawWaker, RawWakerVTable, Waker};
use std::sync::Arc;

// 信号量结构体
pub struct Signal {
    wakers: Arc<Vec<Waker>>,
}

impl Signal {
    // 创建新的Signal实例
    pub fn new() -> Self {
        Signal {
            wakers: Arc::new(Vec::new()),
        }
    }

    // 唤醒所有关联的waker
    pub fn notify(&self) {
        for waker in self.wakers.iter() {
            waker.wake_by_ref();
        }
    }

    // 等待信号
    pub fn wait(&self) {
        // Do nothing
    }

    // 创建一个新的Arc<Waker>并将其加入到wakers列表中
    pub fn add_waker(&mut self, waker: Waker) {
        self.wakers.push(waker);
    }
}

unsafe fn clone_raw(ptr: *const ()) -> RawWaker {
    let arc = Arc::from_raw(ptr as *const Vec<Waker>);
    std::mem::forget(arc.clone());
    
    RawWaker::new(
        ptr,
        &VTABLE,
    )
}

unsafe fn wake_raw(ptr: *const ()) {
    let _arc = Arc::from_raw(ptr as *const Vec<Waker>);
}

unsafe fn wake_by_ref_raw(ptr: *const ()) {
    let arc = Arc::from_raw(ptr as *const Vec<Waker>);
    let waker = arc.iter().next().unwrap().clone();
    waker.wake_by_ref();
}

unsafe fn drop_raw(ptr: *const ()) {
    let _arc = Arc::from_raw(ptr as *const Vec<Waker>);
}

const VTABLE: RawWakerVTable = RawWakerVTable::new(
    clone_raw,
    wake_raw,
    wake_by_ref_raw,
    drop_raw,
);

// 创建一个Waker结构体
pub fn create_waker(signal: &Signal) -> Waker {
    let arc = Arc::new(signal.wakers.clone());
    let raw_waker = RawWaker::new(
        Arc::into_raw(arc) as *const (),
        &VTABLE,
    );
    
    unsafe { Waker::from_raw(raw_waker) }
}
