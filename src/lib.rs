use std::{
    collections::BTreeSet, io::{self, Error, ErrorKind}
};

use tokio::net::{ToSocketAddrs, UdpSocket};

const MAX_LOST_SIZE: usize = 10;
const DATA_SZ: usize = 512;
const FRAME_SZ: usize = 8 + 4 + 2 + DATA_SZ;

struct PktState {
    pkt_id: u64,
    pkt: Vec<u8>,
    set: BTreeSet<u16>,
}

impl PktState {
    fn new(pkt_id: u64, pkt_sz: u32, frame_no: u16, data: &[u8]) -> Self {
        // pkt
        let mut pkt = Vec::with_capacity(pkt_sz as usize);
        pkt.resize(pkt_sz as usize, 0);
        // set
        let mut set = BTreeSet::new();
        let frame_num = (pkt_sz as usize / DATA_SZ + (1 >> pkt_sz as usize % DATA_SZ ^ 1)) as u16;
        for i in 0..frame_num {
            set.insert(i);
        }
        // this
        let mut this = Self { pkt_id, pkt, set };
        this.insert(frame_no, data);
        this
    }

    fn insert(&mut self, frame_no: u16, data: &[u8]) {
        if !self.set.contains(&frame_no) {
            return;
        }
        let remainder = self.pkt.len() % DATA_SZ;
        if remainder > 0 && frame_no == (self.pkt.len() / DATA_SZ) as u16 {
            // There is a remainder and it is the last frame
            self.pkt[frame_no as usize * DATA_SZ..].copy_from_slice(&data[0..remainder]);
        } else {
            self.pkt[frame_no as usize * DATA_SZ..(frame_no + 1) as usize * DATA_SZ]
                .copy_from_slice(data);
        }
        self.set.remove(&frame_no);
    }
}

pub struct PktServer {
    sock: UdpSocket,
}

impl PktServer {
    pub async fn listen<A>(addr: A) -> io::Result<Self>
    where
        A: ToSocketAddrs,
    {
        let sock = UdpSocket::bind(addr).await?;
        Ok(Self { sock })
    }

    pub async fn accept(&self) -> io::Result<PktConn> {
        let mut buf = [0; FRAME_SZ];
        let (_, addr) = self.sock.recv_from(&mut buf).await?;

        let sock = UdpSocket::bind("0.0.0.0:0").await?;
        sock.connect(addr).await?;
        sock.send(&buf).await?;
        Ok(PktConn { sock, pkt_id: 0 })
    }
}

pub struct PktConn {
    sock: UdpSocket,
    pkt_id: u64,
}

impl PktConn {
    pub async fn connect<A>(addr: A) -> io::Result<Self>
    where
        A: ToSocketAddrs,
    {
        let mut buf = [0; FRAME_SZ];
        let sock = UdpSocket::bind("0.0.0.0:0").await?;
        sock.send_to(&buf, addr).await?;
        let (_, addr) = sock.recv_from(&mut buf).await?;
        sock.connect(addr).await?;
        Ok(Self { sock, pkt_id: 0 })
    }

    pub async fn send(&mut self, pkt: &[u8]) -> io::Result<()> {
        self.pkt_id += 1;
        let pkt_sz = pkt.len() as u32;

        let mut buf = [0; FRAME_SZ];
        buf[0..8].copy_from_slice(&self.pkt_id.to_be_bytes());
        buf[8..12].copy_from_slice(&pkt_sz.to_be_bytes());

        let mut p = 0usize;
        let mut q = std::cmp::min(DATA_SZ, pkt_sz as usize);
        let mut frame_no = 0u16;
        while p < q {
            buf[12..14].copy_from_slice(&frame_no.to_be_bytes());
            buf[14..(q - p) + 14].copy_from_slice(&pkt[p..q]);
            self.sock.send(&buf).await?;

            frame_no += 1;
            p = q;
            q += std::cmp::min(DATA_SZ, pkt_sz as usize - p);
        }

        while let Ok(_) = self.sock.recv(&mut buf).await {
            if false == self.retransmit(&mut buf, pkt_sz, &pkt).await? {
                break;
            }
        }
        Ok(())
    }

    pub async fn recv(&self) -> io::Result<Vec<u8>> {
        let mut buf = [0; FRAME_SZ];

        self.sock.recv(&mut buf).await?;
        let pkt_id: u64 = buf[0..8].iter().fold(0, |acc, &b| acc << 8 | b as u64);
        let pkt_sz: u32 = buf[8..12].iter().fold(0, |acc, &b| acc << 8 | b as u32);
        let frame_no: u16 = buf[12..14].iter().fold(0, |acc, &b| acc << 8 | b as u16);

        let mut pkt_state = PktState::new(pkt_id, pkt_sz, frame_no, &buf[14..]);

        let mut retry_times = MAX_LOST_SIZE;
        while !pkt_state.set.is_empty() {
            self.pull(&mut buf, &mut pkt_state, &mut retry_times)
                .await?;
        }
        buf[0..8].copy_from_slice(&pkt_state.pkt_id.to_be_bytes());
        buf[8..12].copy_from_slice(&0u32.to_be_bytes());
        self.sock.send(&buf).await?;

        Ok(pkt_state.pkt)
    }
}

impl PktConn {
    async fn retransmit(&self, buf: &mut [u8], pkt_sz: u32, pkt: &[u8]) -> io::Result<bool> {
        let f_pkt_id = buf[0..8].iter().fold(0, |acc, &b| acc << 8 | b as u64);
        if f_pkt_id != self.pkt_id {
            return Ok(true);
        }
        let f_pkt_sz = buf[8..12].iter().fold(0, |acc, &b| acc << 8 | b as u32);
        if f_pkt_sz == 0 {
            return Ok(false);
        }
        let frame_no = buf[12..14].iter().fold(0, |acc, &b| acc << 8 | b as u16);
        let p = frame_no as usize * DATA_SZ;
        let q = p + std::cmp::min(DATA_SZ, pkt_sz as usize - p);
        buf[12..14].copy_from_slice(&frame_no.to_be_bytes());
        buf[14..(q - p) + 14].copy_from_slice(&pkt[p..q]);
        self.sock.send(&buf).await?;
        return Ok(true);
    }

    async fn request_retransmission(
        &self,
        retry_times: &mut usize,
        pkt_state: &PktState,
        buf: &mut [u8],
    ) -> io::Result<()> {
        *retry_times -= 1;
        if *retry_times == 0 {
            // Too many times retried
            return Err(Error::new(ErrorKind::Other, "Too many times retried"));
        }
        if pkt_state.pkt.len() > MAX_LOST_SIZE {
            // Too many frames lost
            return Err(Error::new(ErrorKind::Other, "Too many frames lost"));
        }

        for no in &pkt_state.set {
            buf[0..8].copy_from_slice(&pkt_state.pkt_id.to_be_bytes());
            buf[8..12].copy_from_slice(&(pkt_state.pkt.len() as u32).to_be_bytes());
            buf[12..14].copy_from_slice(&no.to_be_bytes());
            self.sock.send(&buf).await?;
        }

        Ok(())
    }

    async fn pull(
        &self,
        buf: &mut [u8],
        pkt_state: &mut PktState,
        retry_times: &mut usize,
    ) -> io::Result<()> {
        match self.sock.recv(buf).await {
            Ok(sz) => {
                if sz != FRAME_SZ {
                    return Ok(());
                }
                let pkt_id = buf[0..8].iter().fold(0, |acc, &b| acc << 8 | b as u64);
                let pkt_sz = buf[8..12].iter().fold(0, |acc, &b| acc << 8 | b as u32);
                if pkt_id != pkt_state.pkt_id || pkt_sz != pkt_state.pkt.len() as u32 {
                    return Ok(());
                }
                let frame_no = buf[12..14].iter().fold(0, |acc, &b| acc << 8 | b as u16);
                pkt_state.insert(frame_no, &buf[14..]);
                Ok(())
            }
            Err(e) => {
                if e.kind() == ErrorKind::TimedOut {
                    self.request_retransmission(retry_times, &pkt_state, buf)
                        .await
                } else {
                    Err(e)
                }
            }
        }
    }
}
