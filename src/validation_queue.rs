//! 有界、單一消費者驗證佇列；滿載立即交還工作，不阻塞轉檔。
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};

pub struct Queue<T> {
    sender: Option<SyncSender<T>>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl<T: Send + 'static> Queue<T> {
    pub fn new(
        capacity: usize,
        mut consume: impl FnMut(T) + Send + 'static,
    ) -> std::io::Result<Self> {
        let (sender, receiver) = sync_channel(capacity);
        let worker = std::thread::Builder::new()
            .name("iat-validation".into())
            .spawn(move || {
                for work in receiver {
                    consume(work);
                }
            })?;
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
        })
    }
    pub fn submit(&self, work: T) -> Result<(), T> {
        match self.sender.as_ref().expect("open queue").try_send(work) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(work) | TrySendError::Disconnected(work)) => Err(work),
        }
    }
    pub fn finish(mut self) -> std::thread::Result<()> {
        self.sender.take();
        self.worker.take().expect("queue worker").join()
    }
}
impl<T> Drop for Queue<T> {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{
            Arc, Barrier,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };
    #[test]
    #[ignore = "Actual conversion runtime and iat; set IMBC_TEST_ENGINE and IMBC_TEST_IAT"]
    fn runtime_conversions_finish_while_validation_consumer_waits() {
        use rayon::prelude::*;
        use std::{path::PathBuf, sync::atomic::AtomicBool};
        let engine = PathBuf::from(std::env::var_os("IMBC_TEST_ENGINE").unwrap());
        let exe = PathBuf::from(std::env::var_os("IMBC_TEST_IAT").unwrap());
        let client =
            crate::iat::Client::discover(Some(&exe), &std::env::current_exe().unwrap()).unwrap();
        let root = tempfile::tempdir().unwrap();
        let input = root.path().join("source.png");
        std::fs::write(
            &input,
            include_bytes!("../tests/fixtures/acceptance-v13/icc8.png"),
        )
        .unwrap();
        let (entered, ready) = std::sync::mpsc::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let (results, verified) = std::sync::mpsc::channel();
        let queue = Queue::new(2, move |(input, output): (PathBuf, PathBuf)| {
            entered.send(()).unwrap();
            wait.recv().unwrap();
            let outcome = client.compare(&input, &output, &AtomicBool::new(false));
            assert_eq!(outcome.code, Some(0), "{outcome}");
            results.send(()).unwrap();
        })
        .unwrap();
        // Occupy the sole consumer before real engine work starts.
        queue.submit((input.clone(), input.clone())).unwrap();
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(2)
            .build()
            .unwrap();
        let barrier = Barrier::new(2);
        pool.install(|| {
            (0..2).into_par_iter().for_each(|n| {
                let folder = root.path().join(n.to_string());
                std::fs::create_dir(&folder).unwrap();
                barrier.wait();
                let (output, _) = crate::engine::convert_with_options(
                    &engine,
                    &input,
                    &folder,
                    crate::engine::Format::Png,
                    &crate::engine::Options::default(),
                    &AtomicBool::new(false),
                )
                .unwrap();
                assert!(output.is_file());
                queue.submit((input.clone(), output)).unwrap();
            })
        });
        assert!(verified.try_recv().is_err());
        for _ in 0..3 {
            release.send(()).unwrap();
        }
        queue.finish().unwrap();
        assert_eq!(verified.iter().count(), 3);
    }
    #[test]
    fn two_conversion_workers_continue_while_validation_waits() {
        let (entered, ready) = std::sync::mpsc::channel();
        let (release, wait) = std::sync::mpsc::channel();
        let processed = Arc::new(AtomicUsize::new(0));
        let count = processed.clone();
        let queue = Queue::new(2, move |_: usize| {
            entered.send(()).unwrap();
            wait.recv().unwrap();
            count.fetch_add(1, Ordering::SeqCst);
        })
        .unwrap();
        queue.submit(0).unwrap();
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        let barrier = Barrier::new(2);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(2)
            .build()
            .unwrap();
        use rayon::prelude::*;
        pool.install(|| {
            (1..=2).into_par_iter().for_each(|n| {
                barrier.wait(); // Both conversion workers must actually execute concurrently.
                queue.submit(n).unwrap();
            })
        });
        assert_eq!(processed.load(Ordering::SeqCst), 0);
        assert_eq!(queue.submit(3), Err(3)); // bounded, no producer wait
        for _ in 0..3 {
            release.send(()).unwrap();
        }
        queue.finish().unwrap();
        assert_eq!(processed.load(Ordering::SeqCst), 3);
    }
    #[test]
    fn closed_consumer_returns_work_and_finish_reports_panic() {
        let queue = Queue::new(0, |_: ()| {}).unwrap();
        assert!(queue.submit(()).is_err());
        queue.finish().unwrap();
        let queue = Queue::new(1, |_: ()| panic!("fixture")).unwrap();
        queue.submit(()).unwrap();
        assert!(queue.finish().is_err());
    }
}
