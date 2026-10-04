//! 网格强迫的记录读取：文件句柄复用 + 后台预读下一条记录。
//!
//! 全球强迫文件（如 JRA3Q）按 `(1, nlat, nlon)` 分块：每条记录一整张全球场压成一块，读区域里
//! 任何一个格点都要整块解压。2°×2° 的区域算例主线程一半以上的时间在这里（第 568 轮）。模式本来就
//! 每条记录只读一次，所以省不掉解压，只能让它与物理计算重叠：主线程拿到第 k 条记录后，后台线程
//! 接着读同一文件的第 k+1 条，主线程下一次要它时直接取走（还没读完就等）。
//!
//! **数值不变。** 后台线程做的是与主线程完全相同的 `get_values` 调用（同一窗口、同一类型转换），
//! 只是提前做、换个线程做。netcdf crate 用一把全局锁（`hdf5_sys::LOCK`）串起所有 netCDF/HDF5 调用，
//! 后台读与主线程写历史不会同时进库。
//!
//! 以前每读一条记录都 `netcdf::open` 一次那个几百 MB 的文件；现在两边各自缓存打开的句柄。

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;

use anyhow::{anyhow, Context, Result};

/// 同时打开的文件数上限：超过就全关了重开（按月分文件时一次只用十来个）。
const OPEN_FILES: usize = 32;

/// 读窗口：纬度、经度闭区间（0 起）。
pub(super) type Window = ((usize, usize), (usize, usize));

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    path: PathBuf,
    name: String,
    /// 1 起。
    record: usize,
}

struct Request {
    key: Key,
    window: Window,
}

#[derive(Default)]
struct State {
    queue: VecDeque<Request>,
    pending: HashSet<Key>,
    done: HashMap<Key, std::result::Result<Vec<f64>, String>>,
    shutdown: bool,
}

struct Shared {
    state: Mutex<State>,
    ready: Condvar,
}

/// 见模块文档。
pub(super) struct RecordReader {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    files: Mutex<HashMap<PathBuf, netcdf::File>>,
}

impl std::fmt::Debug for RecordReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordReader").finish_non_exhaustive()
    }
}

impl RecordReader {
    pub(super) fn new() -> Self {
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            ready: Condvar::new(),
        });
        let worker_shared = Arc::clone(&shared);
        let worker = std::thread::Builder::new()
            .name("colm-forcing-prefetch".to_owned())
            .spawn(move || prefetch_loop(&worker_shared))
            .ok();
        Self {
            shared,
            worker,
            files: Mutex::new(HashMap::new()),
        }
    }

    /// 读 `path` 里变量 `name` 的第 `record` 条（1 起）在 `window` 上的块（按 `(lat, lon)` 行主序），
    /// 并请后台预读下一条。
    pub(super) fn read_block(
        &self,
        path: &Path,
        name: &str,
        record: usize,
        window: Window,
    ) -> Result<Vec<f64>> {
        let key = Key {
            path: path.to_path_buf(),
            name: name.to_owned(),
            record,
        };
        let prefetched = {
            let mut state = self.shared.state.lock().expect("forcing reader lock");
            // 同一变量别的预读结果都过时了（换了文件或记录往回走），丢掉。
            state.done.retain(|other, _| {
                other.name != key.name || (other.path == key.path && other.record >= record)
            });
            loop {
                if let Some(result) = state.done.remove(&key) {
                    break Some(result);
                }
                if !state.pending.contains(&key) {
                    break None;
                }
                state = self.shared.ready.wait(state).expect("forcing reader lock");
            }
        };
        let block = match prefetched {
            Some(result) => result.map_err(|message| anyhow!(message))?,
            None => {
                let mut files = self.files.lock().expect("forcing reader file lock");
                read_with(&mut files, path, name, record, window)?
            }
        };
        if self.worker.is_some() {
            let next = Key {
                path: key.path,
                name: key.name,
                record: record + 1,
            };
            let mut state = self.shared.state.lock().expect("forcing reader lock");
            if !state.pending.contains(&next) && !state.done.contains_key(&next) {
                state.pending.insert(next.clone());
                state.queue.push_back(Request { key: next, window });
                self.shared.ready.notify_all();
            }
        }
        Ok(block)
    }
}

impl Drop for RecordReader {
    fn drop(&mut self) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.shutdown = true;
        }
        self.shared.ready.notify_all();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn prefetch_loop(shared: &Shared) {
    let mut files = HashMap::new();
    loop {
        let request = {
            let mut state = shared.state.lock().expect("forcing reader lock");
            loop {
                if state.shutdown {
                    return;
                }
                if let Some(request) = state.queue.pop_front() {
                    break request;
                }
                state = shared.ready.wait(state).expect("forcing reader lock");
            }
        };
        let Request { key, window } = request;
        // 文件末尾之后没有下一条：不读，也不留结果（主线程会去读下一个文件）。
        let result = match record_count(&mut files, &key.path, &key.name) {
            Ok(count) if key.record > count => None,
            Ok(_) => Some(
                read_with(&mut files, &key.path, &key.name, key.record, window)
                    .map_err(|error| format!("{error:#}")),
            ),
            Err(error) => Some(Err(format!("{error:#}"))),
        };
        let mut state = shared.state.lock().expect("forcing reader lock");
        state.pending.remove(&key);
        if let Some(result) = result {
            state.done.insert(key, result);
        }
        shared.ready.notify_all();
    }
}

fn open<'a>(
    files: &'a mut HashMap<PathBuf, netcdf::File>,
    path: &Path,
) -> Result<&'a netcdf::File> {
    if !files.contains_key(path) {
        if files.len() >= OPEN_FILES {
            files.clear();
        }
        let file = netcdf::open(path)
            .with_context(|| format!("cannot open the forcing file {}", path.display()))?;
        files.insert(path.to_path_buf(), file);
    }
    Ok(files.get(path).expect("just inserted"))
}

fn record_count(
    files: &mut HashMap<PathBuf, netcdf::File>,
    path: &Path,
    name: &str,
) -> Result<usize> {
    let file = open(files, path)?;
    let variable = file
        .variable(name)
        .with_context(|| format!("{} has no variable {name}", path.display()))?;
    Ok(variable
        .dimensions()
        .first()
        .map_or(0, netcdf::Dimension::len))
}

fn read_with(
    files: &mut HashMap<PathBuf, netcdf::File>,
    path: &Path,
    name: &str,
    record: usize,
    window: Window,
) -> Result<Vec<f64>> {
    let file = open(files, path)?;
    let variable = file
        .variable(name)
        .with_context(|| format!("{} has no variable {name}", path.display()))?;
    let ((lat0, lat1), (lon0, lon1)) = window;
    variable
        .get_values((record - 1, lat0..=lat1, lon0..=lon1))
        .with_context(|| format!("cannot read {name} record {record} from {}", path.display()))
}

#[cfg(test)]
#[path = "forcing_reader_tests.rs"]
mod forcing_reader_tests;
