use std::{
    collections::BTreeSet,
    io::{self, ErrorKind, Error},
    net::{ToSocketAddrs, UdpSocket},
    time::Duration,
};

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
        let mut pkt = Vec::with_capacity(pkt_sz as usize);
        pkt.resize(pkt_sz as usize, 0);
        let mut set = BTreeSet::new();
        let frame_num = (pkt_sz as usize / DATA_SZ + (1 >> pkt_sz as usize % DATA_SZ ^ 1)) as u16;
        for i in 0..frame_num {
            set.insert(i);
        }
        let mut this = Self { pkt_id, pkt, set };
        this.insert(frame_no, data);
        this
    }

    fn insert(&mut self, frame_no: u16, data: &[u8]) {
        if !self.set.contains(&frame_no) {
            return;
        }
        let rest = self.pkt.len() % DATA_SZ;
        if rest > 0 && frame_no == (self.pkt.len() / DATA_SZ) as u16 {
            self.pkt[frame_no as usize * DATA_SZ..]
                .copy_from_slice(&data[0..rest]);
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
    pub fn listen<A>(addr: A) -> io::Result<Self>
    where
        A: ToSocketAddrs,
    {
        let sock = UdpSocket::bind(addr)?;
        Ok(Self { sock })
    }

    pub fn accept(&self) -> io::Result<PktConn> {
        let mut buf = [0; FRAME_SZ];
        let (_, addr) = self.sock.recv_from(&mut buf)?;

        let sock = UdpSocket::bind("0.0.0.0:0")?;
        sock.connect(addr)?;
        sock.send(&buf)?;
        sock.set_read_timeout(Some(Duration::new(0, 460_000)))?;
        Ok(PktConn { sock, pkt_id: 0 })
    }
}

pub struct PktConn {
    sock: UdpSocket,
    pkt_id: u64,
}

impl PktConn {
    pub fn connect<A>(addr: A) -> io::Result<Self>
    where
        A: ToSocketAddrs,
    {
        let mut buf = [0; FRAME_SZ];
        let sock = UdpSocket::bind("0.0.0.0:0")?;
        sock.send_to(&buf, addr)?;
        let (_, addr) = sock.recv_from(&mut buf)?;
        sock.connect(addr)?;
        sock.set_read_timeout(Some(Duration::new(0, 460_000)))?;
        Ok(Self { sock, pkt_id: 0 })
    }

    pub fn send(&mut self, pkt: &[u8]) -> io::Result<()> {
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
            self.sock.send(&buf)?;

            frame_no += 1;
            p = q;
            q += std::cmp::min(DATA_SZ, pkt_sz as usize - p);
        }

        let mut f_pkt_id: u64;
        let mut f_pkt_sz: u32;
        while let Ok(_) = self.sock.recv(&mut buf) {
            f_pkt_id = buf[0..8].iter().fold(0, |acc, &b| acc << 8 | b as u64);
            if f_pkt_id != self.pkt_id {
                continue;
            }
            f_pkt_sz = buf[8..12].iter().fold(0, |acc, &b| acc << 8 | b as u32);
            if f_pkt_sz == 0 {
                break;
            }
            frame_no = buf[12..14].iter().fold(0, |acc, &b| acc << 8 | b as u16);
            p = frame_no as usize * DATA_SZ;
            q = p + std::cmp::min(DATA_SZ, pkt_sz as usize - p);
            buf[12..14].copy_from_slice(&frame_no.to_be_bytes());
            buf[14..(q - p) + 14].copy_from_slice(&pkt[p..q]);
            self.sock.send(&buf)?;
        }
        Ok(())
    }

    pub fn recv(&self) -> io::Result<Vec<u8>> {
        let mut buf = [0; FRAME_SZ];

        self.sock.recv(&mut buf)?;
        let mut pkt_id: u64 = buf[0..8].iter().fold(0, |acc, &b| acc << 8 | b as u64);
        let mut pkt_sz: u32 = buf[8..12].iter().fold(0, |acc, &b| acc << 8 | b as u32);
        let mut frame_no: u16 = buf[12..14].iter().fold(0, |acc, &b| acc << 8 | b as u16);

        let mut pkt_state = PktState::new(pkt_id, pkt_sz, frame_no, &buf[14..]);

        let mut retry_times = MAX_LOST_SIZE;
        while !pkt_state.set.is_empty() {
            match self.sock.recv(&mut buf) {
                Ok(_) => {
                    pkt_id = buf[0..8].iter().fold(0, |acc, &b| acc << 8 | b as u64);
                    pkt_sz = buf[8..12].iter().fold(0, |acc, &b| acc << 8 | b as u32);
                    if pkt_id != pkt_state.pkt_id || pkt_sz != pkt_state.pkt.len() as u32 {
                        continue;
                    }
                    frame_no = buf[12..14].iter().fold(0, |acc, &b| acc << 8 | b as u16);
                    pkt_state.insert(frame_no, &buf[14..]);
                }
                Err(e) => {
                    if e.kind() == ErrorKind::TimedOut {
                        retry_times -= 1;
                        if retry_times == 0 {
                            return Err(Error::new(ErrorKind::Other, "too many frames lost"));
                        }
                        if pkt_state.pkt.len() > MAX_LOST_SIZE {
                            return Err(Error::new(ErrorKind::Other, "too many frames lost"));
                        }

                        for no in &pkt_state.set {
                            buf[0..8].copy_from_slice(&pkt_state.pkt_id.to_be_bytes());
                            buf[8..12].copy_from_slice(&(pkt_state.pkt.len() as u32).to_be_bytes());
                            buf[12..14].copy_from_slice(&no.to_be_bytes());
                            self.sock.send(&buf)?;
                        }
                    } else {
                        return Err(e);
                    }
                }
            }
        }
        buf[0..8].copy_from_slice(&pkt_state.pkt_id.to_be_bytes());
        buf[8..12].copy_from_slice(&0u32.to_be_bytes());
        self.sock.send(&buf)?;

        Ok(pkt_state.pkt)
    }
}
