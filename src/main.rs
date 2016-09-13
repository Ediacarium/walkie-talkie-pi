extern crate bincode;
extern crate rustc_serialize;
#[macro_use]
extern crate log;
extern crate env_logger;
extern crate rand;

mod packet_layer;

use packet_layer::packet_layer;
use rand::Rng;
use std::io;
use std::io::BufRead;

fn main() {
    
    env_logger::init().unwrap();
    let mut rng = rand::thread_rng();
    
    let port = 1337;
	let addr = rng.gen();
	
	let (mut tx,rx) = packet_layer(port,addr).unwrap();
	tx.send("Hello World".to_string().into_bytes());
	
	use std::thread;

	thread::spawn(move || {
	loop {
		let (payload, address, sequence_number) = rx.receive().unwrap();
		println!("{},{}: {}",address, sequence_number, String::from_utf8(payload).unwrap());
	}
	});
	
	loop {
		let stdin = io::stdin();
	    for line in stdin.lock().lines() {
        	tx.send(line.unwrap().into_bytes());
    	}
	}
	
}
