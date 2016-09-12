extern crate bincode;
extern crate rustc_serialize;
#[macro_use]
extern crate log;
extern crate env_logger;
extern crate rand;

mod packet_layer;

use std::net::UdpSocket;
use packet_layer::PacketLayer;
use std::net::Ipv4Addr;
use std::str::FromStr;
use log::LogLevel;
use rand::Rng;

fn main() {
    
    env_logger::init().unwrap();
    let mut rng = rand::thread_rng();
    
    let port = 1337;
    let ip_addr = "0.0.0.0";
    let broadcast = "255.255.255.255";
	let socket = UdpSocket::bind((ip_addr,port)).unwrap();
	socket.set_broadcast(true);
	socket.join_multicast_v4(&Ipv4Addr::from_str(broadcast).unwrap(),&Ipv4Addr::from_str(ip_addr).unwrap());
	//println!("{}",socket.broadcast().unwrap());
	let addr = rng.gen();
	let mut packet_layer = PacketLayer::new(port,addr,socket);
	packet_layer.send("lululululul".to_string().into_bytes());
    loop {
    	let (payload, _, _ ) = packet_layer.receive().unwrap();
    	println!("Received: {}", String::from_utf8(payload).unwrap());
    }
}
