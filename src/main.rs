extern crate bincode;
extern crate rustc_serialize;
#[macro_use]
extern crate log;

mod packet_layer;

use std::net::UdpSocket;
use packet_layer::PacketLayer;
use std::net::Ipv4Addr;
use std::str::FromStr;

fn main() {
    
    let port = 1337;
    let ip_addr = "192.168.43.142";
    let broadcast = "192.168.43.255";
	let mut socket = UdpSocket::bind((ip_addr,1338)).unwrap();
	socket.set_broadcast(true);
	socket.join_multicast_v4(&Ipv4Addr::from_str(broadcast).unwrap(),&Ipv4Addr::from_str(ip_addr).unwrap());
	//println!("{}",socket.broadcast().unwrap());
	let mut addr = 322420133742;
	let mut packet_layer = PacketLayer::new(port,addr,socket);
	packet_layer.send("lululululul".to_string().into_bytes());
    loop {
    	let (payload, _, _ ) = packet_layer.receive().unwrap();
    	println!("Received: {}", String::from_utf8(payload).unwrap());
    }
}
