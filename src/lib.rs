use std::{
    io,
    net::{ToSocketAddrs, UdpSocket},
};

const DATA_SZ: usize = 512;
const FRAME_SZ: usize = 8 + 4 + 2 + DATA_SZ;

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
        Ok(())
    }

    pub fn recv(&self) -> io::Result<Vec<u8>> {
        let mut buf = [0; FRAME_SZ];

        self.sock.recv(&mut buf)?;
        let pkt_id: u64 = buf[0..8].iter().fold(0, |acc, &b| acc << 8 | b as u64);
        let pkt_sz: u32 = buf[8..12].iter().fold(0, |acc, &b| acc << 8 | b as u32);
        let mut frame_no: u16 = buf[12..14].iter().fold(0, |acc, &b| acc << 8 | b as u16);

        let frame_num = pkt_sz / DATA_SZ as u32 + if pkt_sz % DATA_SZ as u32 > 0 { 1 } else { 0 };
        let mut pkt = Vec::new();
        pkt.resize(pkt_sz as usize, 0);
        if frame_no < frame_num as u16 - 1 {
            pkt[frame_no as usize * DATA_SZ..(frame_no as usize + 1) * DATA_SZ]
                .copy_from_slice(&buf[14..]);
        } else {
            pkt[frame_no as usize * DATA_SZ..].copy_from_slice(
                &buf[14..14
                    + if pkt_sz as usize % DATA_SZ > 0 {
                        pkt_sz as usize % DATA_SZ
                    } else {
                        DATA_SZ
                    }],
            );
        }

        let mut f_pkt_id: u64;
        let mut f_pkt_sz: u32;
        let mut i = 1;
        while i < frame_num {
            self.sock.recv(&mut buf)?;
            f_pkt_id = buf[0..8].iter().fold(0, |acc, &b| acc << 8 | b as u64);
            f_pkt_sz = buf[8..12].iter().fold(0, |acc, &b| acc << 8 | b as u32);

            if f_pkt_id != pkt_id || f_pkt_sz != pkt_sz {
                continue;
            }

            frame_no = buf[12..14].iter().fold(0, |acc, &b| acc << 8 | b as u16);
            if frame_no < frame_num as u16 - 1 {
                pkt[frame_no as usize * DATA_SZ..(frame_no as usize + 1) * DATA_SZ]
                    .copy_from_slice(&buf[14..]);
            } else {
                pkt[frame_no as usize * DATA_SZ..].copy_from_slice(
                    &buf[14..14
                        + if pkt_sz as usize % DATA_SZ > 0 {
                            pkt_sz as usize % DATA_SZ
                        } else {
                            DATA_SZ
                        }],
                );
            }
            i += 1;
        }

        Ok(pkt)
    }
}
