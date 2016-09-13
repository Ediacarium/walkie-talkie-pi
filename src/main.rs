extern crate bincode;
extern crate rustc_serialize;
#[macro_use]
extern crate log;
extern crate env_logger;
extern crate rand;

mod packet_layer;

use packet_layer::packet_layer;
use rand::Rng;

fn main() {
    
    env_logger::init().unwrap();
    let mut rng = rand::thread_rng();
    
    let port = 1337;
    let ip_addr = "0.0.0.0";
	
	let addr = rng.gen();
	
	let (mut tx,rx) = packet_layer(port,addr).unwrap();
	tx.send("Hello World".to_string().into_bytes());
	loop {
		let (payload, _, _) = rx.receive().unwrap();
		println!("Received: {}", String::from_utf8(payload).unwrap());
	}
}
