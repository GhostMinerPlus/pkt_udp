use std::io;

fn main() -> io::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let server = pkt_udp::PktServer::listen("0.0.0.0:9004").await?;
            let mut buf = [0; 513];
            for i in 0..buf.len() {
                buf[i] = (i % 256) as u8;
            }
            while let Ok(mut conn) = server.accept().await {
                conn.send(&buf).await?;
            }
            Ok(())
        })
}
