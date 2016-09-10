//use self::bincode::SizeLimit;

static PACKET_HEADER_ID : i16 = 0x0ADC;
static ADVERTISEMENT_PACKET_TYPE : i8= 0;
static SEND_REQUEST_PACKET_TYPE : i8= 1;
static PAYLOAD_PACKET_TYPE : i8= 2;


use bincode::SizeLimit;
use rustc_serialize::Encodable;
use std::vec::Vec;
use std::clone::Clone;
use std::hash::Hash;
use std::collections::HashMap;
use std::sync::mpsc::{channel,Sender,Receiver};
use std::thread;
use std::thread::{Thread,JoinHandle};
use std::net::UdpSocket;
use bincode::rustc_serialize::{encode, decode};
use std::sync::mpsc::TryRecvError;
use std::sync::mpsc::RecvError;

#[derive(RustcEncodable, RustcDecodable, PartialEq, Eq, Clone, Hash)]
pub struct PacketHeader {
	identifier: i16 ,
	packet_type: i8,
}

#[derive(RustcEncodable, RustcDecodable, PartialEq, Eq, Clone, Hash)]
pub struct PacketId {
	source_ip_addr: i64,
	sequence_number: u8
}

#[derive(RustcEncodable, RustcDecodable, PartialEq, Eq, Clone, Hash)]
pub struct AdvertisementPacket {
	header: PacketHeader,
	packet: PacketId,
	advertiser: i64
}

#[derive(RustcEncodable, RustcDecodable, PartialEq, Eq, Clone, Hash)]
pub struct SendRequestPacket {
	header: PacketHeader,
	packet: PacketId,
	favoured_sender: i64
}

#[derive(RustcEncodable, RustcDecodable, PartialEq, Eq, Clone, Hash)]
pub struct PayloadPacket {
	header: PacketHeader,
	packet: PacketId,
	payload: Vec<u8>
}
#[derive(RustcEncodable, RustcDecodable, PartialEq)]
enum SendablePackets {
	AdvertisementPacket(AdvertisementPacket),
	SendRequestPacket(SendRequestPacket),
	PayloadPacket(PayloadPacket)
}

pub struct PacketLayer {
	ip_address: i64,
	port : u16,
	sequence_number: u8,
	worker: (JoinHandle<()>,Sender<PayloadPacket>,Receiver<PayloadPacket>),
	socket: UdpSocket
}

impl PacketHeader {
	pub fn new(packet_type: i8) -> Self {
		PacketHeader {
			identifier: PACKET_HEADER_ID,
			packet_type: packet_type
		}
	}
}

impl PacketId {
	pub fn new(source_ip_addr: i64, sequence_number: u8)-> Self {
		PacketId {
			source_ip_addr: source_ip_addr,
			sequence_number: sequence_number
		}
	}
}

impl AdvertisementPacket {
	pub fn new(packet: &PayloadPacket, advertiser: i64) -> Self {
		AdvertisementPacket{
			header: PacketHeader::new(ADVERTISEMENT_PACKET_TYPE),
			packet: packet.packet.clone(),
			advertiser: advertiser
		}
	}
}

impl SendRequestPacket {
	pub fn new(packet: &AdvertisementPacket) -> Self{
		SendRequestPacket {
			header: PacketHeader::new(SEND_REQUEST_PACKET_TYPE),
			packet: packet.packet.clone(),
			favoured_sender: packet.advertiser
		}
	}
}

impl PayloadPacket {
	pub fn new(payload: &Vec<u8>, source_ip_addr: i64, sequence_number: u8) -> Self{
		PayloadPacket {
			header: PacketHeader::new(PAYLOAD_PACKET_TYPE),
			packet: PacketId::new(source_ip_addr, sequence_number),
			payload: payload.to_vec()
		}
	}
}


impl PacketLayer {
	pub fn new(port: u16, ip_address:i64, socket:UdpSocket) -> Self{
		let (send_tx,send_rx) = channel();
		let (receive_tx, receive_rx) = channel();
		let worker_socket = socket.try_clone().unwrap();
		let workthread = thread::spawn(move|| {worker_loop(port,ip_address,worker_socket,send_rx,receive_tx)});
		PacketLayer{
			ip_address : ip_address,
			sequence_number : 0,
			worker : (workthread, send_tx, receive_rx),
			socket : socket,
			port : port
		}
	}
	
	pub fn receive(&self) -> Result<(Vec<u8>,i64,u8),RecvError>{
	
		match self.worker.2.recv() {
			Ok(packet) =>Ok((packet.payload,packet.packet.source_ip_addr,packet.packet.sequence_number)),
			Err(E) => Err(E)
		}
	}
	
	pub fn try_receive(&self) -> Result<(Vec<u8>,i64,u8),TryRecvError> {
		match self.worker.2.try_recv(){
					Ok(packet) =>Ok((packet.payload,packet.packet.source_ip_addr,packet.packet.sequence_number)),
			Err(E) => Err(E)
		}
	}
	
	pub fn send(&mut self, payload: Vec<u8>){
		let packet = PayloadPacket::new(&payload,self.ip_address,self.sequence_number);
		let advertisement = AdvertisementPacket::new(&packet, self.ip_address);
		self.worker.1.send(packet);
		self.socket.send_to(&encode(&advertisement,SizeLimit::Infinite).unwrap(),("192.168.43.255",self.port));
	}
}

fn handle_advertisement(advertisementpacket: AdvertisementPacket, socket:&UdpSocket, id_to_packet: &HashMap<PacketId,PayloadPacket>, port: u16){
	
	if let None =  id_to_packet.get(&advertisementpacket.packet) {

	let sendrequest = SendRequestPacket::new(&advertisementpacket);
	socket.send_to(&encode(&sendrequest, SizeLimit::Infinite).unwrap(), ("192.168.43.255",port));
	}
}

fn handle_send_request(sendrequestpacket: SendRequestPacket, socket: &UdpSocket, id_to_packet: &HashMap<PacketId,PayloadPacket>, ip_address: i64, port: u16) {
	
	if sendrequestpacket.favoured_sender == ip_address {
	
		if let Some(packet) = id_to_packet.get(&sendrequestpacket.packet) {
		socket.send_to(&encode(&packet, SizeLimit::Infinite).unwrap(), ("192.168.43.255", port));
		}		
	}
}

fn handle_payload(payloadpacket: PayloadPacket, socket: &UdpSocket, id_to_packet: &mut HashMap<PacketId,PayloadPacket>, ip_address: i64, port: u16, received: &Sender<PayloadPacket>){
	
	if let None = id_to_packet.get(&payloadpacket.packet) {
			id_to_packet.insert(payloadpacket.packet.clone(),payloadpacket.clone());
			let advertisement = AdvertisementPacket::new(&payloadpacket, ip_address);
			socket.send_to(&encode(&advertisement, SizeLimit::Infinite).unwrap(), ("192.168.43.255", port));
	}
	
	if let Err(failure) = received.send(payloadpacket.clone()) {
		warn!["Cannot forward received PayloadPacket"];
	}
	
}

fn worker_loop(port: u16, ip_address:i64, socket:UdpSocket, send:Receiver<PayloadPacket>, received:Sender<PayloadPacket>) {
	
	let mut buffer = Vec::with_capacity(1500);
	let mut running = true;
	let mut id_to_payload = HashMap::new();
	
	while running {
		socket.recv_from(&mut buffer);
		println!("Message received!");
		//empty message queue
		let mut pending_message = PayloadPacket::new(&Vec::<u8>::with_capacity(0),0,0);
		while match send.try_recv() {
			Ok(s) => {pending_message = s; true},
			Err(e) => {if e == TryRecvError::Disconnected { running = false; }; false}
			} != false {
			let packet_id = pending_message.packet.clone();
			id_to_payload.insert(packet_id,pending_message.clone());
			
		}
		
		match decode(&mut buffer) {
		Err(e) => warn!("Cannot decode recieved Packet. Error: {}",e),
		Ok(packet) => match packet {
			SendablePackets::AdvertisementPacket(adv) => handle_advertisement(adv,&socket,&id_to_payload, port),
			SendablePackets::SendRequestPacket(srp) => handle_send_request(srp,&socket,&id_to_payload, ip_address, port),
			SendablePackets::PayloadPacket(pp) => handle_payload(pp,&socket,&mut id_to_payload, ip_address, port, &received)
			}
		}
	}			
}
	

