use std::io;

fn main() -> io::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let conn = pkt_udp::PktConn::connect("127.0.0.1:9004").await?;
            let r = conn.recv().await?;
            println!("{:?}", &r[0..512]);
            Ok(())
        })
}
