//! 多进程运行的字节传输层：TCP 星形，按 rank 有序的 gather/scatter/broadcast/barrier。
//!
//! **为什么不用 MPI**：0.1° 全球每步约 0.3 GB 的 patch 级交换（见 `docs/design-multi-node.md`
//! §4），星形 TCP 在 10–200 Gb/s 网络上占计算比例很小；而 MPI 要求每个远程构建主机都有 MPI
//! 开发库（tianhe 只有 `mpiexec`、没有 `mpicc`），还会和安装包自带的 `kernels/_runtime`
//! MPI 运行时冲突。这里只用 std，macOS/Windows 本机与任何 Linux 服务器都不需要新工具链。
//!
//! **确定性**：root 永远按 rank 顺序收发，不按到达顺序；上层再把字节按全局 patch 索引重排，
//! 所以传输层不引入额外的非确定性。
//!
//! **启动**：root 监听 `0.0.0.0:0`，把 `host\nport\n` 原子写进 rendezvous 文件；worker 轮询
//! 该文件后连接。rank/size 从显式参数或 `SLURM_*` / `OMPI_COMM_WORLD_*` / `PMI_*` 读出。
//!
//! **消息上限**：16 GiB/条。0.1° 全球的 history 单记录约 8 GB 仍可一次发；再大或要降内存
//! 占用时改分块流式发送（Phase 4 的任务，不在本模块假装支持）。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{bail, ensure, Context, Result};

const MAX_FRAME_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(600);
const CONNECT_RETRY: Duration = Duration::from_millis(50);
const ACK: u8 = 0xA5;

/// 本进程的 rank 与进程总数。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RankInfo {
    rank: usize,
    size: usize,
}

impl RankInfo {
    pub fn new(rank: usize, size: usize) -> Result<Self> {
        ensure!(size >= 1, "a multi-process run needs at least one process");
        ensure!(
            rank < size,
            "rank {rank} is outside 0..{size}; the launcher gave a bad rank count"
        );
        Ok(Self { rank, size })
    }

    /// 显式参数优先，其次按 `SLURM` → `OpenMPI` → `PMI` 的顺序读启动器变量。
    pub fn resolve(rank: Option<usize>, size: Option<usize>) -> Result<Self> {
        Self::resolve_with(rank, size, |name| std::env::var(name).ok())
    }

    /// 与 [`RankInfo::resolve`] 相同，但环境变量由调用方提供（测试用）。
    pub fn resolve_with(
        rank: Option<usize>,
        size: Option<usize>,
        env: impl Fn(&str) -> Option<String>,
    ) -> Result<Self> {
        if let (Some(rank), Some(size)) = (rank, size) {
            return Self::new(rank, size);
        }
        ensure!(
            rank.is_none() && size.is_none(),
            "--rank and --ranks must be given together, or both omitted so the launcher environment decides"
        );
        let found = [
            ("SLURM_PROCID", "SLURM_NTASKS"),
            ("OMPI_COMM_WORLD_RANK", "OMPI_COMM_WORLD_SIZE"),
            ("PMI_RANK", "PMI_SIZE"),
        ]
        .iter()
        .find_map(|(rank_name, size_name)| {
            let rank = env(rank_name)?.parse::<usize>().ok()?;
            let size = env(size_name)?.parse::<usize>().ok()?;
            Some((rank, size))
        })
        .context(
            "no process rank found: start through srun/mpiexec, or pass --rank and --ranks; \
             tried SLURM_PROCID/SLURM_NTASKS, OMPI_COMM_WORLD_RANK/OMPI_COMM_WORLD_SIZE, PMI_RANK/PMI_SIZE",
        )?;
        Self::new(found.0, found.1)
    }

    pub fn rank(&self) -> usize {
        self.rank
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn is_root(&self) -> bool {
        self.rank == 0
    }
}

/// 一个 rank 的通信端。root 持有到所有 worker 的连接；worker 只持有一条到 root 的连接。
pub struct Comms {
    rank: usize,
    size: usize,
    link: Link,
    timeout: Duration,
}

enum Link {
    /// `streams[rank]` 是 rank 的连接；root 自己的位置是 `None`。
    Root(Vec<Option<TcpStream>>),
    /// worker 到 root 的连接。
    Worker(TcpStream),
}

impl Comms {
    /// 按 `info` 决定当 root 还是 worker。`advertise_host` 只对 root 有意义：
    /// 它写进 rendezvous 文件供其它节点连接；`None` 时用 `COLM_SPMD_HOST` / `HOSTNAME` /
    /// `/proc/sys/kernel/hostname`，最后退回 `127.0.0.1`（只适合单节点）。
    pub fn open(rendezvous: &Path, info: RankInfo, advertise_host: Option<&str>) -> Result<Self> {
        Self::open_with_timeout(rendezvous, info, advertise_host, DEFAULT_TIMEOUT)
    }

    pub fn open_with_timeout(
        rendezvous: &Path,
        info: RankInfo,
        advertise_host: Option<&str>,
        timeout: Duration,
    ) -> Result<Self> {
        if info.is_root() {
            Self::root_with_timeout(rendezvous, info.size(), advertise_host, timeout)
        } else {
            Self::worker_with_timeout(rendezvous, info, timeout)
        }
    }

    fn root_with_timeout(
        rendezvous: &Path,
        size: usize,
        advertise_host: Option<&str>,
        timeout: Duration,
    ) -> Result<Self> {
        ensure!(size >= 1, "a multi-process run needs at least one process");
        let listener =
            TcpListener::bind(("0.0.0.0", 0)).context("cannot listen for the other processes")?;
        let port = listener
            .local_addr()
            .context("cannot read the listening address")?
            .port();
        listener
            .set_nonblocking(true)
            .context("cannot set the listener to polling mode")?;
        let host = advertise_host
            .filter(|host| !host.is_empty())
            .map(str::to_owned)
            .or_else(default_host)
            .unwrap_or_else(|| "127.0.0.1".to_owned());
        write_rendezvous(rendezvous, &host, port)?;

        let deadline = Instant::now() + timeout;
        let mut streams = (0..size).map(|_| None).collect::<Vec<_>>();
        let mut connected = 0;
        while connected + 1 < size {
            match listener.accept() {
                Ok((stream, peer)) => {
                    stream
                        .set_nonblocking(false)
                        .with_context(|| format!("cannot make {peer} blocking"))?;
                    stream
                        .set_read_timeout(Some(timeout))
                        .context("cannot set the read timeout")?;
                    stream
                        .set_write_timeout(Some(timeout))
                        .context("cannot set the write timeout")?;
                    let mut stream = stream;
                    let (rank, peer_size) = read_handshake(&mut stream)
                        .with_context(|| format!("handshake with {peer} failed"))?;
                    ensure!(
                        peer_size == size,
                        "rank {rank} from {peer} thinks there are {peer_size} processes, not {size}"
                    );
                    ensure!(
                        rank >= 1 && rank < size,
                        "rank {rank} from {peer} is outside 1..{size}"
                    );
                    ensure!(
                        streams[rank].is_none(),
                        "rank {rank} connected more than once ({peer})"
                    );
                    write_ack(&mut stream).with_context(|| format!("cannot acknowledge {peer}"))?;
                    streams[rank] = Some(stream);
                    connected += 1;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    ensure!(
                        Instant::now() < deadline,
                        "timed out after {} s waiting for {}/{} processes to connect; \
                         check the launcher, the rendezvous file {} and the node network",
                        timeout.as_secs(),
                        connected + 1,
                        size,
                        rendezvous.display()
                    );
                    std::thread::sleep(CONNECT_RETRY);
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("cannot accept a worker connection on port {port}")
                    })
                }
            }
        }
        Ok(Self {
            rank: 0,
            size,
            link: Link::Root(streams),
            timeout,
        })
    }

    fn worker_with_timeout(rendezvous: &Path, info: RankInfo, timeout: Duration) -> Result<Self> {
        let deadline = Instant::now() + timeout;
        let (host, port) = loop {
            match std::fs::read_to_string(rendezvous) {
                Ok(text) => {
                    let mut lines = text.lines();
                    let host = lines.next().unwrap_or_default().trim().to_owned();
                    let port = lines
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .parse::<u16>()
                        .with_context(|| {
                            format!("rendezvous file {} is malformed", rendezvous.display())
                        })?;
                    ensure!(
                        !host.is_empty(),
                        "rendezvous file {} has an empty host",
                        rendezvous.display()
                    );
                    break (host, port);
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    ensure!(
                        Instant::now() < deadline,
                        "timed out after {} s waiting for the rendezvous file {} (is rank 0 running?)",
                        timeout.as_secs(),
                        rendezvous.display()
                    );
                    std::thread::sleep(CONNECT_RETRY);
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("cannot read the rendezvous file {}", rendezvous.display())
                    })
                }
            }
        };
        let address = format!("{host}:{port}");
        let stream = connect_with_retry(&address, deadline, timeout)?;
        let mut stream = stream;
        write_handshake(&mut stream, info.rank(), info.size())?;
        read_ack(&mut stream)
            .with_context(|| format!("rank 0 did not acknowledge rank {}", info.rank()))?;
        Ok(Self {
            rank: info.rank(),
            size: info.size(),
            link: Link::Worker(stream),
            timeout,
        })
    }

    pub fn rank(&self) -> usize {
        self.rank
    }

    pub fn size(&self) -> usize {
        self.size
    }

    pub fn is_root(&self) -> bool {
        self.rank == 0
    }

    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// 各 rank 的字节拼起来，按 rank 顺序；root 拿到 `Some(全局)`，worker 拿到 `None`。
    ///
    /// 这是逐位一致的关键接口：root 不能按到达顺序拼，worker 也不能在别人之前走。
    pub fn gather(&mut self, local: &[u8]) -> Result<Option<Vec<u8>>> {
        match &mut self.link {
            Link::Root(streams) => {
                let mut global = Vec::with_capacity(local.len());
                global.extend_from_slice(local);
                for (rank, stream) in streams.iter_mut().enumerate().skip(1) {
                    let stream = stream
                        .as_mut()
                        .with_context(|| format!("rank {rank} is not connected"))?;
                    let frame = read_frame(stream)
                        .with_context(|| format!("cannot receive gather data from rank {rank}"))?;
                    global.extend_from_slice(&frame);
                }
                for (rank, stream) in streams.iter_mut().enumerate().skip(1) {
                    let stream = stream
                        .as_mut()
                        .with_context(|| format!("rank {rank} is not connected"))?;
                    write_frame(stream, &[])
                        .with_context(|| format!("cannot release rank {rank} after gather"))?;
                }
                Ok(Some(global))
            }
            Link::Worker(stream) => {
                write_frame(stream, local)?;
                let ack = read_frame(stream).context("cannot read the gather acknowledgement")?;
                ensure!(ack.is_empty(), "the gather acknowledgement carries data");
                Ok(None)
            }
        }
    }

    /// root 按 `chunk_lengths` 切 `global` 发给各 rank；worker 只收自己的那一份。
    ///
    /// 参数形态不同是故意的：worker 不该知道全局缓冲，也不该反推自己的长度。
    pub fn scatter(
        &mut self,
        global: Option<&[u8]>,
        chunk_lengths: Option<&[usize]>,
    ) -> Result<Vec<u8>> {
        match &mut self.link {
            Link::Root(streams) => {
                let global = global.context("root must pass the global scatter buffer")?;
                let lengths =
                    chunk_lengths.context("root must pass the per-rank scatter lengths")?;
                ensure!(
                    lengths.len() == self.size,
                    "scatter has {} chunk lengths for {} processes",
                    lengths.len(),
                    self.size
                );
                ensure!(
                    lengths.iter().sum::<usize>() == global.len(),
                    "scatter chunk lengths add up to {}, not {}",
                    lengths.iter().sum::<usize>(),
                    global.len()
                );
                let mut offset = 0;
                let mut own = Vec::new();
                for (rank, &length) in lengths.iter().enumerate() {
                    let chunk = &global[offset..offset + length];
                    offset += length;
                    if rank == 0 {
                        own = chunk.to_vec();
                    } else {
                        let stream = streams[rank]
                            .as_mut()
                            .with_context(|| format!("rank {rank} is not connected"))?;
                        write_frame(stream, chunk)
                            .with_context(|| format!("cannot send scatter data to rank {rank}"))?;
                    }
                }
                Ok(own)
            }
            Link::Worker(stream) => {
                ensure!(
                    global.is_none() && chunk_lengths.is_none(),
                    "a worker must not pass global scatter buffers"
                );
                read_frame(stream).context("cannot receive scatter data from root")
            }
        }
    }

    /// root 把自己的字节发给所有 worker；worker 拿到 root 的那份。
    pub fn broadcast(&mut self, value: Option<&[u8]>) -> Result<Vec<u8>> {
        match &mut self.link {
            Link::Root(streams) => {
                let value = value.context("root must pass the broadcast value")?;
                for (rank, stream) in streams.iter_mut().enumerate().skip(1) {
                    let stream = stream
                        .as_mut()
                        .with_context(|| format!("rank {rank} is not connected"))?;
                    write_frame(stream, value)
                        .with_context(|| format!("cannot broadcast to rank {rank}"))?;
                }
                Ok(value.to_vec())
            }
            Link::Worker(stream) => read_frame(stream).context("cannot read the broadcast value"),
        }
    }

    /// 等所有 rank 到达。用空 gather 实现，root 仍按 rank 顺序收。
    pub fn barrier(&mut self) -> Result<()> {
        let _ = self.gather(&[])?;
        Ok(())
    }
}

fn default_host() -> Option<String> {
    if let Ok(host) = std::env::var("COLM_SPMD_HOST") {
        if !host.trim().is_empty() {
            return Some(host.trim().to_owned());
        }
    }
    if let Ok(host) = std::env::var("HOSTNAME") {
        if !host.trim().is_empty() {
            return Some(host.trim().to_owned());
        }
    }
    std::fs::read_to_string("/proc/sys/kernel/hostname")
        .ok()
        .map(|host| host.trim().to_owned())
        .filter(|host| !host.is_empty())
}

fn write_rendezvous(path: &Path, host: &str, port: u16) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "cannot create the rendezvous directory {}",
                parent.display()
            )
        })?;
    }
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&temporary, format!("{host}\n{port}\n"))
        .with_context(|| format!("cannot write {}", temporary.display()))?;
    std::fs::rename(&temporary, path)
        .with_context(|| format!("cannot publish the rendezvous file {}", path.display()))?;
    Ok(())
}

fn connect_with_retry(address: &str, deadline: Instant, timeout: Duration) -> Result<TcpStream> {
    loop {
        match TcpStream::connect(address) {
            Ok(stream) => {
                stream
                    .set_read_timeout(Some(timeout))
                    .context("cannot set the read timeout")?;
                stream
                    .set_write_timeout(Some(timeout))
                    .context("cannot set the write timeout")?;
                return Ok(stream);
            }
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
                ensure!(
                    Instant::now() < deadline,
                    "timed out after {} s connecting to {address}",
                    timeout.as_secs()
                );
                std::thread::sleep(CONNECT_RETRY);
            }
            Err(error) => {
                return Err(error).with_context(|| format!("cannot connect to {address}"))
            }
        }
    }
}

fn write_handshake(stream: &mut TcpStream, rank: usize, size: usize) -> Result<()> {
    let mut bytes = Vec::with_capacity(8);
    bytes.extend_from_slice(
        &u32::try_from(rank)
            .context("rank does not fit in u32")?
            .to_le_bytes(),
    );
    bytes.extend_from_slice(
        &u32::try_from(size)
            .context("process count does not fit in u32")?
            .to_le_bytes(),
    );
    write_frame(stream, &bytes)
}

fn read_handshake(stream: &mut TcpStream) -> Result<(usize, usize)> {
    let frame = read_frame(stream)?;
    ensure!(
        frame.len() == 8,
        "handshake must be 8 bytes, got {}",
        frame.len()
    );
    let rank = u32::from_le_bytes(frame[0..4].try_into().expect("4 bytes")) as usize;
    let size = u32::from_le_bytes(frame[4..8].try_into().expect("4 bytes")) as usize;
    Ok((rank, size))
}

fn write_ack(stream: &mut TcpStream) -> Result<()> {
    stream
        .write_all(&[ACK])
        .context("cannot write the ack byte")?;
    stream.flush().context("cannot flush the ack byte")
}

fn read_ack(stream: &mut TcpStream) -> Result<()> {
    let mut ack = [0u8; 1];
    stream
        .read_exact(&mut ack)
        .context("cannot read the ack byte")?;
    ensure!(ack[0] == ACK, "the peer sent an unexpected handshake byte");
    Ok(())
}

fn write_frame(stream: &mut TcpStream, payload: &[u8]) -> Result<()> {
    let length = payload.len() as u64;
    ensure!(
        length <= MAX_FRAME_BYTES,
        "message of {length} bytes exceeds the {} byte limit",
        MAX_FRAME_BYTES
    );
    stream
        .write_all(&length.to_le_bytes())
        .context("cannot write a message length")?;
    stream
        .write_all(payload)
        .context("cannot write a message")?;
    stream.flush().context("cannot flush a message")
}

fn read_frame(stream: &mut TcpStream) -> Result<Vec<u8>> {
    let mut length = [0u8; 8];
    stream
        .read_exact(&mut length)
        .context("cannot read a message length")?;
    let length = u64::from_le_bytes(length);
    ensure!(
        length <= MAX_FRAME_BYTES,
        "peer announced {length} bytes, above the {} byte limit",
        MAX_FRAME_BYTES
    );
    let length = usize::try_from(length).context("message length does not fit in usize")?;
    let mut payload = vec![0u8; length];
    stream
        .read_exact(&mut payload)
        .context("cannot read a message body")?;
    Ok(payload)
}

/// `f64` 切片 → 小端字节；按数组次序，不做任何重排。
pub fn encode_f64s(values: &[f64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 8);
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

/// [`encode_f64s`] 的逆操作。
pub fn decode_f64s(bytes: &[u8]) -> Result<Vec<f64>> {
    ensure!(
        bytes.len() % 8 == 0,
        "f64 payload of {} bytes is not a multiple of 8",
        bytes.len()
    );
    Ok(bytes
        .chunks_exact(8)
        .map(|chunk| f64::from_le_bytes(chunk.try_into().expect("8 bytes")))
        .collect())
}

/// `u64` 切片 → 小端字节；分片表、长度表这类整数序列用它。
pub fn encode_u64s(values: &[u64]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * 8);
    for value in values {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

/// [`encode_u64s`] 的逆操作。
pub fn decode_u64s(bytes: &[u8]) -> Result<Vec<u64>> {
    ensure!(
        bytes.len() % 8 == 0,
        "u64 payload of {} bytes is not a multiple of 8",
        bytes.len()
    );
    Ok(bytes
        .chunks_exact(8)
        .map(|chunk| u64::from_le_bytes(chunk.try_into().expect("8 bytes")))
        .collect())
}

/// `bool` 切片 → 0/1 字节。
pub fn encode_bools(values: &[bool]) -> Vec<u8> {
    values.iter().map(|value| u8::from(*value)).collect()
}

/// [`encode_bools`] 的逆操作；只接受 0/1。
pub fn decode_bools(bytes: &[u8]) -> Result<Vec<bool>> {
    bytes
        .iter()
        .map(|byte| match byte {
            0 => Ok(false),
            1 => Ok(true),
            other => bail!("bool payload contains {other}, not 0 or 1"),
        })
        .collect()
}

#[cfg(test)]
#[path = "lib_tests.rs"]
mod lib_tests;
