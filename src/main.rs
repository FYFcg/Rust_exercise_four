use futures::future::BoxFuture;
use futures::FutureExt;
use std::cell::RefCell;
use std::collections::VecDeque;
use std::future::Future;
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Wake, Waker};
use std::pin::Pin;

struct Demo;

impl Future for Demo {
    type Output = ();

    fn poll(self:Pin<&mut Self>,_cx: &mut std::task::Context<'_>) -> std::task::Poll<Self::Output> {
        println!("Hello World!");
        std::task::Poll::Ready(())
    }
}

pub struct Task {
    future: RefCell<BoxFuture<'static, ()>>,
    signal: Signal,
}

scoped_tls::scoped_thread_local!(static SIGNAL: Arc<Signal>);
scoped_tls::scoped_thread_local!(static RUNNABLE: Mutex<VecDeque<Arc<Task>>>);

unsafe impl Send for Task {}
unsafe impl Sync for Task {}

impl Wake for Task {
    fn wake(self: Arc<Self>) {
        RUNNABLE.with(|runnable| runnable.lock().unwrap().push_back(self.clone()));
        self.signal.notify();
    }
}

fn multi_block_on<F: Future>(future: F) -> F::Output {
    let mut fut = std::pin::pin!(future);
    let signal: Arc<Signal> = Arc::new(Signal::new());
    let waker = Waker::from(signal.clone());

    let mut cx = Context::from_waker(&waker);
    let runnable: Mutex<VecDeque<Arc<Task>>> = Mutex::new(VecDeque::with_capacity(1024));
    SIGNAL.set(&signal, || {
        RUNNABLE.set(&runnable, || loop {
            if let Poll::Ready(output) = fut.as_mut().poll(&mut cx) {
                return output;
            }
            while let Some(task) = runnable.lock().unwrap().pop_front() {
                let waker = Waker::from(task.clone());
                let mut cx = Context::from_waker(&waker);
                let _ = task.future.borrow_mut().as_mut().poll(&mut cx);
            }
            signal.wait();
        })
    })
}

#[allow(unused)]
fn ori_block_on<F: Future>(future: F) -> F::Output {
    let mut fut = std::pin::pin!(future);
    // let signal: Arc<Signal> = Arc::new(Signal::new());
    let waker = dummy_waker();

    let mut cx = Context::from_waker(&waker);
    loop {
        if let Poll::Ready(output) = fut.as_mut().poll(&mut cx) {
            return output;
        }
        // signal.wait();
    }
}

#[allow(unused)]
fn block_on<F: Future>(future: F) -> F::Output {
    let mut fut = std::pin::pin!(future);
    let signal: Arc<Signal> = Arc::new(Signal::new());
    let waker = Waker::from(signal.clone());

    let mut cx = Context::from_waker(&waker);
    loop {
        if let Poll::Ready(output) = fut.as_mut().poll(&mut cx) {
            return output;
        }
        signal.wait();
    }
}

fn dummy_waker() -> Waker {
    static DATA: () = ();
    unsafe { Waker::from_raw(RawWaker::new(&DATA, &VTABLE)) }
}

struct Signal {
    state: Mutex<State>,
    cond: Condvar,
}

enum State {
    Empty,
    Waiting,
    Notified,
}

impl Signal {
    fn new() -> Signal {
        Signal {
            state: Mutex::new(State::Empty),
            cond: Condvar::new(),
        }
    }
    fn wait(&self) {
        let mut state = self.state.lock().unwrap();
        match *state {
            State::Notified => *state = State::Empty,
            State::Waiting => {
                panic!("multiple wait");
            }
            State::Empty => {
                *state = State::Waiting;
                while let State::Waiting = *state {
                    state = self.cond.wait(state).unwrap();
                }
            }
        }
    }

    fn notify(&self) {
        let mut state = self.state.lock().unwrap();
        match *state {
            State::Notified => {}
            State::Waiting => {
                *state = State::Empty;
                self.cond.notify_one();
            }
            State::Empty => *state = State::Notified,
        }
    }
}

const VTABLE: RawWakerVTable =
    RawWakerVTable::new(vtable_clone, vtable_wake, vtable_wake_by_ref, vtable_drop);

unsafe fn vtable_clone(_p: *const ()) -> RawWaker {
    RawWaker::new(_p, &VTABLE)
}

unsafe fn vtable_wake(_p: *const ()) {}

unsafe fn vtable_wake_by_ref(_p: *const ()) {}

unsafe fn vtable_drop(_p: *const ()) {}

fn spawn(fut: impl Future<Output = ()> + 'static + std::marker::Send) {
    let t = Arc::new(Task {
        future: RefCell::new(fut.boxed()),
        signal: Signal::new(),
    });
    RUNNABLE.with(|runnable| runnable.lock().unwrap().push_back(t));
}

async fn demo() {
    let (tx, rx) = async_channel::bounded::<()>(2);
    spawn(demo2(tx));
    println!("Hello World");
    let _ = rx.recv().await;
}

async fn demo2(tx: async_channel::Sender<()>) {
    println!("Hello world2");
    let _ = tx.send(()).await;
}

impl Wake for Signal {
    fn wake(self: Arc<Self>) {
        self.notify();
    }
}

fn main() {
    multi_block_on(demo())
}
