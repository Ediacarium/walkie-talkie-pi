//use self::bincode::SizeLimit;

use bincode::SizeLimit;
use std::vec::Vec;
use std::clone::Clone;
use std::collections::HashMap;
use std::sync::mpsc::{channel,Sender,Receiver};
use std::thread;
use std::thread::{JoinHandle};
use std::net::UdpSocket;
use bincode::rustc_serialize::{encode, decode};
use std::sync::mpsc::TryRecvError;
use std::sync::mpsc::RecvError;


#[derive(RustcEncodable, RustcDecodable, PartialEq, Eq, Clone, Hash)]
pub struct PacketId {
	source_ip_addr: i64,
	sequence_number: u8
}

#[derive(RustcEncodable, RustcDecodable, PartialEq, Eq, Clone, Hash)]
pub struct AdvertisementPacket {
	packet: PacketId,
	advertiser: i64
}

#[derive(RustcEncodable, RustcDecodable, PartialEq, Eq, Clone, Hash)]
pub struct SendRequestPacket {
	packet: PacketId,
	favoured_sender: i64
}

#[derive(RustcEncodable, RustcDecodable, PartialEq, Eq, Clone, Hash)]
pub struct PayloadPacket {
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
			packet: packet.packet.clone(),
			advertiser: advertiser
		}
	}
}

impl SendRequestPacket {
	pub fn new(packet: &AdvertisementPacket) -> Self{
		SendRequestPacket {
			packet: packet.packet.clone(),
			favoured_sender: packet.advertiser
		}
	}
}

impl PayloadPacket {
	pub fn new(payload: &Vec<u8>, source_ip_addr: i64, sequence_number: u8) -> Self{
		PayloadPacket {
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
		let packet = try!(self.worker.2.recv());
		Ok((packet.payload,packet.packet.source_ip_addr,packet.packet.sequence_number))
	}
	
	pub fn try_receive(&self) -> Result<(Vec<u8>,i64,u8),TryRecvError> {
		let packet = try!(self.worker.2.try_recv());
		Ok((packet.payload,packet.packet.source_ip_addr,packet.packet.sequence_number))
	}
	
	pub fn send(&mut self, payload: Vec<u8>){
		debug!("got new payload to send");
		let packet = PayloadPacket::new(&payload,self.ip_address,self.sequence_number);
		let advertisement = SendablePackets::AdvertisementPacket(AdvertisementPacket::new(&packet, self.ip_address));
		self.worker.1.send(packet);
		
		let advertisement_encoded = &encode(&advertisement, SizeLimit::Infinite).unwrap();
		debug!("sending {} Bytes",advertisement_encoded.len());
		self.socket.send_to(&advertisement_encoded,("255.255.255.255",self.port));
	}
}

fn handle_advertisement(advertisementpacket: AdvertisementPacket, socket:&UdpSocket, id_to_packet: &HashMap<PacketId,PayloadPacket>, port: u16){

	info!("handling advertisement packet");
	if let None =  id_to_packet.get(&advertisementpacket.packet) {
		
		debug!("Haven't received Payload Packet yet, sending send request");	
		let sendrequest = SendablePackets::SendRequestPacket(SendRequestPacket::new(&advertisementpacket));
		socket.send_to(&encode(&sendrequest, SizeLimit::Infinite).unwrap(), ("255.255.255.255",port));
	}
	else{
		debug!("Already got advertised Packet, ignoring request.");
	}
}

fn handle_send_request(sendrequestpacket: SendRequestPacket, socket: &UdpSocket, id_to_packet: &HashMap<PacketId,PayloadPacket>, ip_address: i64, port: u16) {
	
	info!("handling send request packet");
	if sendrequestpacket.favoured_sender == ip_address {
	
		debug!("send request is targeted at us, sending packet");
		if let Some(packet) = id_to_packet.get(&sendrequestpacket.packet) {
		socket.send_to(&encode(&SendablePackets::PayloadPacket(packet.clone()), SizeLimit::Infinite).unwrap(), ("255.255.255.255", port));
		}		
	}
	else{
		debug!("send request is not targeted at us, ignoring");
	}
}

fn handle_payload(payloadpacket: PayloadPacket, socket: &UdpSocket, id_to_packet: &mut HashMap<PacketId,PayloadPacket>, ip_address: i64, port: u16, received: &Sender<PayloadPacket>){
	
	info!("handling payload packet");
	if let None = id_to_packet.get(&payloadpacket.packet) {
			
			debug!("haven't gotten payload packet, saving");
			id_to_packet.insert(payloadpacket.packet.clone(),payloadpacket.clone());
			let advertisement = SendablePackets::AdvertisementPacket(AdvertisementPacket::new(&payloadpacket, ip_address));
			let advertisement_encoded = &encode(&advertisement, SizeLimit::Infinite).unwrap();
			info!("sending {} Bytes : {:?}",advertisement_encoded.len(), advertisement_encoded);
			socket.send_to(advertisement_encoded, ("255.255.255.255", port));
	}
	else{
		debug!("already received payload packet, ignoring");
	}
	
	if let Err(_) = received.send(payloadpacket.clone()) {
		warn!("Cannot forward received PayloadPacket");
	}
	
}

fn worker_loop(port: u16, ip_address:i64, socket:UdpSocket, send:Receiver<PayloadPacket>, received:Sender<PayloadPacket>) {
	
	info!("worker started!");
	let mut buffer = [0;1500];
	let mut running = true;
	let mut id_to_payload = HashMap::new();
	
	while running {
		if let Ok(amount) = socket.recv_from(&mut buffer){
			debug!("Received a message from the socket (length: {})",amount.0);
			//empty message queue
			let mut pending_message = PayloadPacket::new(&Vec::<u8>::with_capacity(0),0,0);
			while match send.try_recv() {
				Ok(s) => {pending_message = s; true},
				Err(e) => {if e == TryRecvError::Disconnected { running = false; }; false}
				} != false {
				debug!("Got a pending payload package from the channel");
				let packet_id = pending_message.packet.clone();
				id_to_payload.insert(packet_id,pending_message.clone());
			}
		
			match decode(&buffer[0..amount.0]) {
			Err(e) => error!("Cannot decode recieved Packet. Error: {}",e),
			Ok(packet) => match packet {
				SendablePackets::AdvertisementPacket(adv) => handle_advertisement(adv,&socket,&id_to_payload, port),
				SendablePackets::SendRequestPacket(srp) => handle_send_request(srp,&socket,&id_to_payload, ip_address, port),
				SendablePackets::PayloadPacket(pp) => handle_payload(pp,&socket,&mut id_to_payload, ip_address, port, &received)
				}
			}
		}
	}	
	debug!("worker thread exited");		
}
	

