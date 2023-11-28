use std::io;

fn main() -> io::Result<()> {
    let server = pkt_udp::PktServer::listen("0.0.0.0:9004")?;
    let mut buf = [0; 513];
    for i in 0..buf.len() {
        buf[i] = (i % 256) as u8;
    }
    while let Ok(mut conn) = server.accept() {
        conn.send(&buf)?;
    }
    Ok(())
}
