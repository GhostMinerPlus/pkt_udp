use std::io;

fn main() -> io::Result<()> {
    let conn = pkt_udp::PktConn::connect("127.0.0.1:9004")?;
    let r = conn.recv()?;
    println!("{:?}", &r[0..512]);
    Ok(())
}
