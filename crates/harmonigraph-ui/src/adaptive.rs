//! Live next-attack neighbourhood. The worker owns analytic winner enumeration;
//! camera, display fades and rendering never enter its cache key.
use harmonigraph_core::{
    policy::reach::{self, Snapshot},
    LatticePos,
};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc, Arc,
};
struct Request {
    generation: u64,
    snapshot: Snapshot,
}
pub struct Neighbourhood {
    input: Option<Snapshot>,
    pending: Option<Request>,
    generation: Arc<AtomicU64>,
    tx: mpsc::SyncSender<Request>,
    rx: mpsc::Receiver<(u64, Result<Vec<LatticePos>, String>)>,
    pub nodes: Vec<LatticePos>,
    pub computing: bool,
    pub error: Option<String>,
    pub visible: bool,
}
impl Default for Neighbourhood {
    fn default() -> Self {
        let (tx, requests) = mpsc::sync_channel::<Request>(1);
        let (results, rx) = mpsc::channel();
        let generation = Arc::new(AtomicU64::new(0));
        let current = generation.clone();
        std::thread::Builder::new()
            .name("tuning-neighbourhood".into())
            .spawn(move || {
                while let Ok(request) = requests.recv() {
                    let cancel = || current.load(Ordering::Relaxed) != request.generation;
                    if cancel() {
                        continue;
                    }
                    let result = reach::reachable(&request.snapshot, 3600.0, 9600.0, cancel)
                        .map_err(|e| format!("{e:?}"));
                    if !cancel() && results.send((request.generation, result)).is_err() {
                        break;
                    }
                }
            })
            .expect("neighbourhood worker");
        Self {
            input: None,
            pending: None,
            generation,
            tx,
            rx,
            nodes: Vec::new(),
            computing: false,
            error: None,
            visible: true,
        }
    }
}
impl Drop for Neighbourhood {
    fn drop(&mut self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }
}
impl Neighbourhood {
    pub fn update(&mut self, snapshot: Snapshot) {
        if self.input.as_ref() != Some(&snapshot) {
            self.input = Some(snapshot.clone());
            let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
            self.pending = Some(Request { generation, snapshot });
            self.nodes.clear();
            self.error = None;
            self.computing = true;
        }
        if let Some(request) = self.pending.take() {
            if let Err(mpsc::TrySendError::Full(request)) = self.tx.try_send(request) {
                self.pending = Some(request);
            }
        }
        while let Ok((generation, result)) = self.rx.try_recv() {
            if generation != self.generation.load(Ordering::Relaxed) {
                continue;
            }
            self.computing = false;
            match result {
                Ok(nodes) => self.nodes = nodes,
                Err(error) => self.error = Some(error),
            }
        }
    }
    pub fn has_context(&self) -> bool {
        self.input.is_some()
    }
}
