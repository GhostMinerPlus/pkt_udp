use std::io;

fn main() -> io::Result<()> {
    let server = pkt_udp::PktServer::listen("0.0.0.0:9004")?;
    let buf = [0; 65536];
    while let Ok(mut conn) = server.accept() {
        conn.send(&buf)?;
    }
    Ok(())
}
