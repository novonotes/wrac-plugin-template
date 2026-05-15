//! `novonotes_run_loop` 上で繰り返し callback を実行する timer。
//!
//! host callback に頼らず GUI thread 側で定期処理したい場合に使う。callback に
//! `Send` を要求しないため、native GUI object や WebView channel のような
//! thread-affine な値を扱いやすい。

use std::{
    cell::{Cell, RefCell},
    future::Future,
    rc::{Rc, Weak},
    time::Duration,
};

use novonotes_run_loop::{Handle, RunLoop};

trait TimerCallback: 'static {
    fn execute(&self, run_loop: &RunLoop);
}

struct SyncCallback<F>
where
    F: Fn() + 'static,
{
    callback: F,
}

impl<F> TimerCallback for SyncCallback<F>
where
    F: Fn() + 'static,
{
    fn execute(&self, _run_loop: &RunLoop) {
        (self.callback)();
    }
}

struct AsyncCallback<F, Fut>
where
    F: Fn() -> Fut + 'static,
    Fut: Future<Output = ()> + 'static,
{
    callback: F,
}

impl<F, Fut> TimerCallback for AsyncCallback<F, Fut>
where
    F: Fn() -> Fut + 'static,
    Fut: Future<Output = ()> + 'static,
{
    fn execute(&self, run_loop: &RunLoop) {
        run_loop.spawn((self.callback)());
    }
}

struct TimerInner {
    is_running: Cell<bool>,
    current_handle: RefCell<Option<Handle>>,
    interval: Duration,
    callback: Box<dyn TimerCallback>,
}

/// RunLoop thread 専用の繰り返し timer。
///
/// 作成、開始、停止、破棄は同じ run loop thread 上で行う前提です。`stop()` または
/// drop により次回の schedule は cancel されます。
///
/// RunLoop の timer cadence は OS や実行環境に依存するため、この型は指定 interval
/// どおりの発火回数や高精度な周期を保証しません。GUI の状態反映など、遅延や間引きを
/// 許容できる用途を想定しています。
pub struct Timer {
    inner: Rc<TimerInner>,
}

impl Timer {
    pub fn new<F>(interval: Duration, callback: F) -> Self
    where
        F: Fn() + 'static,
    {
        Self {
            inner: Rc::new(TimerInner {
                is_running: Cell::new(false),
                current_handle: RefCell::new(None),
                interval,
                callback: Box::new(SyncCallback { callback }),
            }),
        }
    }

    /// Callback-local state を持つ timer を簡潔に作る helper。
    ///
    /// 現在の template 本体では使っていないが、plugin ごとの GUI polling や軽量な
    /// debounce state を追加するときに `Rc<RefCell<_>>` の boilerplate を利用者へ
    /// 書かせないために public API として残している。
    pub fn new_with_state<T, F>(interval: Duration, initial_state: T, callback: F) -> Self
    where
        T: 'static,
        F: Fn(&mut T) + 'static,
    {
        let state = Rc::new(RefCell::new(initial_state));
        let state_for_callback = state.clone();
        Self::new(interval, move || {
            callback(&mut state_for_callback.borrow_mut());
        })
    }

    pub fn new_async<F, Fut>(interval: Duration, callback: F) -> Self
    where
        F: Fn() -> Fut + 'static,
        Fut: Future<Output = ()> + 'static,
    {
        Self {
            inner: Rc::new(TimerInner {
                is_running: Cell::new(false),
                current_handle: RefCell::new(None),
                interval,
                callback: Box::new(AsyncCallback { callback }),
            }),
        }
    }

    pub fn start(&self) {
        debug_assert!(
            RunLoop::try_current().is_ok(),
            "Timer must be started on the initialized RunLoop thread"
        );

        if self.inner.is_running.replace(true) {
            return;
        }

        self.schedule_next();
    }

    pub fn stop(&self) {
        self.inner.is_running.set(false);

        if let Some(mut handle) = self.inner.current_handle.borrow_mut().take() {
            handle.cancel();
        }
    }

    pub fn is_running(&self) -> bool {
        self.inner.is_running.get()
    }

    fn schedule_next(&self) {
        let weak_inner = Rc::downgrade(&self.inner);
        let interval = self.inner.interval;

        let handle = RunLoop::current().schedule(interval, move || {
            run_timer_tick(weak_inner);
        });

        *self.inner.current_handle.borrow_mut() = Some(handle);
    }
}

fn run_timer_tick(weak_inner: Weak<TimerInner>) {
    let Some(inner) = weak_inner.upgrade() else {
        return;
    };

    if !inner.is_running.get() {
        return;
    }

    let run_loop = RunLoop::current();
    inner.callback.execute(&run_loop);

    let interval = inner.interval;
    let weak_inner_for_next = weak_inner.clone();
    let handle = run_loop.schedule(interval, move || {
        run_timer_tick(weak_inner_for_next);
    });

    *inner.current_handle.borrow_mut() = Some(handle);
}

impl Drop for Timer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use novonotes_run_loop::test_helper::run_async;
    use serial_test::serial;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

    #[test]
    #[serial]
    fn timer_state_tracks_start_and_stop() {
        run_async(async {
            let timer = Timer::new(Duration::from_millis(100), || {});

            assert!(!timer.is_running());
            timer.start();
            assert!(timer.is_running());
            timer.stop();
            assert!(!timer.is_running());
        });
    }

    #[test]
    #[serial]
    fn stop_prevents_future_ticks_after_observed_fire() {
        run_async(async {
            let counter = Arc::new(AtomicU32::new(0));
            let counter_clone = counter.clone();

            let timer = Timer::new(Duration::from_millis(100), move || {
                counter_clone.fetch_add(1, Ordering::SeqCst);
            });

            timer.start();
            RunLoop::current().delay(Duration::from_millis(150)).await;
            timer.stop();
            let count_at_stop = counter.load(Ordering::SeqCst);
            RunLoop::current().delay(Duration::from_millis(200)).await;

            // The exact count is not part of the Timer contract. Headless CI and
            // GUI hosts may run the underlying RunLoop timers at different cadences.
            assert!(count_at_stop >= 1, "timer did not fire before stop");
            assert_eq!(counter.load(Ordering::SeqCst), count_at_stop);
        });
    }

    #[test]
    #[serial]
    fn async_task_continues_after_timer_drop() {
        run_async(async {
            let task_started = Arc::new(AtomicBool::new(false));
            let task_completed = Arc::new(AtomicBool::new(false));
            let task_started_clone = task_started.clone();
            let task_completed_clone = task_completed.clone();

            {
                let timer = Timer::new_async(Duration::from_millis(20), move || {
                    let started = task_started_clone.clone();
                    let completed = task_completed_clone.clone();
                    async move {
                        started.store(true, Ordering::SeqCst);
                        RunLoop::current().delay(Duration::from_millis(200)).await;
                        completed.store(true, Ordering::SeqCst);
                    }
                });

                timer.start();
                wait_until_true(&task_started).await;
            }

            // Dropping the timer must not cancel a task that has already been spawned, but the
            // completion delay is still driven by the platform run loop. Wait for the observable
            // condition instead of assuming CI delivers the 200ms delay within a fixed margin.
            wait_until_true(&task_completed).await;
        });
    }

    #[test]
    #[serial]
    #[should_panic(expected = "Timer must be started on the initialized RunLoop thread")]
    #[cfg(debug_assertions)]
    fn timer_panics_without_run_loop_in_debug() {
        let timer = Timer::new(Duration::from_millis(100), || {});
        timer.start();
    }

    async fn wait_until_true(flag: &AtomicBool) {
        // Observe the event instead of assuming a fixed timer cadence. CFRunLoop
        // timers can run slower in headless CI or host-specific GUI environments.
        for _ in 0..50 {
            if flag.load(Ordering::SeqCst) {
                return;
            }
            RunLoop::current().delay(Duration::from_millis(20)).await;
        }

        panic!("condition was not observed before timeout");
    }
}
