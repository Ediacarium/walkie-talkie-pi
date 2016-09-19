//use self::bincode::SizeLimit;

use bincode::SizeLimit;
use std::vec::Vec;
use std::clone::Clone;
use std::collections::HashMap;
use std::sync::mpsc::{channel, Sender, Receiver};
use std::thread;
use std::thread::{JoinHandle};
use bincode::rustc_serialize::{encode, decode};
use std::sync::mpsc::TryRecvError;
use std::sync::mpsc::RecvError;
use std::sync::Arc;
use std::net::UdpSocket;
use std::net::Ipv4Addr;
use std::str::FromStr;
use std::io::Error as IOError;

static IP_ADDR_ANY : &'static str = "0.0.0.0";
static BROADCAST_ALL : &'static str = "255.255.255.255";


#[derive(RustcEncodable, RustcDecodable, PartialEq, Eq, Clone, Hash)]
pub struct PacketId {
    source_ip_addr: i64,
    sequence_number: u8,
}

impl PacketId {
    pub fn new(source_ip_addr: i64, sequence_number: u8) -> Self {
        PacketId {
            source_ip_addr: source_ip_addr,
            sequence_number: sequence_number,
        }
    }
}

#[derive(RustcEncodable, RustcDecodable, PartialEq, Eq, Clone, Hash)]
pub struct AdvertisementPacket {
    packet: PacketId,
    advertiser: i64,
}

impl AdvertisementPacket {
    pub fn new(packet: &PayloadPacket, advertiser: i64) -> Self {
        AdvertisementPacket {
            packet: packet.packet.clone(),
            advertiser: advertiser,
        }
    }
}

#[derive(RustcEncodable, RustcDecodable, PartialEq, Eq, Clone, Hash)]
pub struct SendRequestPacket {
    packet: PacketId,
    favoured_sender: i64,
}

impl SendRequestPacket {
    pub fn new(packet: &AdvertisementPacket) -> Self {
        SendRequestPacket {
            packet: packet.packet.clone(),
            favoured_sender: packet.advertiser,
        }
    }
}

#[derive(RustcEncodable, RustcDecodable, PartialEq, Eq, Clone, Hash)]
pub struct PayloadPacket {
    packet: PacketId,
    payload: Vec<u8>,
}

impl PayloadPacket {
    pub fn new(payload: &Vec<u8>, source_ip_addr: i64, sequence_number: u8) -> Self {
        PayloadPacket {
            packet: PacketId::new(source_ip_addr, sequence_number),
            payload: payload.to_vec(),
        }
    }
}

#[derive(RustcEncodable, RustcDecodable, PartialEq)]
enum SendablePackets {
    AdvertisementPacket(AdvertisementPacket),
    SendRequestPacket(SendRequestPacket),
    PayloadPacket(PayloadPacket),
}

pub struct PacketSender {
    ip_address: i64,
    port: u16,
    sequence_number: u8,
    sender: Sender<PayloadPacket>,
    socket: UdpSocket,
    worker: Arc<JoinHandle<()>>,
}

impl PacketSender {
    pub fn send(&mut self, payload: Vec<u8>) {
        debug!("got new payload to send");
        let packet = PayloadPacket::new(&payload, self.ip_address, self.sequence_number);
        let advertisement = SendablePackets::AdvertisementPacket(AdvertisementPacket::new(&packet, self.ip_address));
        self.sender.send(packet);

        let advertisement_encoded = &encode(&advertisement, SizeLimit::Infinite).unwrap();
        debug!("sending {} Bytes", advertisement_encoded.len());
        match self.socket.send_to(&advertisement_encoded, (BROADCAST_ALL, self.port)) {
            Ok(_) => debug!("Successfully sent advertisement!"),
            Err(e) => error!("Failed to send advertisement: {}", e)
        }
        self.sequence_number = self.sequence_number.wrapping_add(1);
        debug!("incremented sequence_number to {}", self.sequence_number);
    }
}

pub struct PacketReceiver {
    receiver: Receiver<PayloadPacket>,
    worker: Arc<JoinHandle<()>>,
}

impl PacketReceiver {
    pub fn receive(&self) -> Result<(Vec<u8>, i64, u8), RecvError> {
        let packet = try!(self.receiver.recv());
        Ok((packet.payload, packet.packet.source_ip_addr, packet.packet.sequence_number))
    }

    pub fn try_receive(&self) -> Result<(Vec<u8>, i64, u8), TryRecvError> {
        let packet = try!(self.receiver.try_recv());
        Ok((packet.payload, packet.packet.source_ip_addr, packet.packet.sequence_number))
    }
}

pub fn packet_layer(port: u16, ip_address:i64) -> Result<(PacketSender, PacketReceiver), IOError> {
    let (send_tx, send_rx) = channel();
    let (receive_tx, receive_rx) = channel();
    let socket = try!(UdpSocket::bind((IP_ADDR_ANY, port)));
    let worker_socket = socket.try_clone().unwrap();
    worker_socket.set_broadcast(true);
    worker_socket.join_multicast_v4(&Ipv4Addr::from_str(BROADCAST_ALL).unwrap(), &Ipv4Addr::from_str(IP_ADDR_ANY).unwrap());
    let workthread = thread::spawn(move|| {worker_loop(port, ip_address, worker_socket, send_rx, receive_tx)});
    let worker = Arc::new(workthread);
    Ok((PacketSender {
        ip_address : ip_address,
        sequence_number : 0,
        socket : socket,
        port : port,
        worker: worker.clone(),
        sender: send_tx,
    },
    PacketReceiver {
        receiver: receive_rx,
        worker : worker,
    }))
}

fn handle_advertisement(advertisementpacket: AdvertisementPacket, socket: &UdpSocket, id_to_packet: &HashMap<PacketId, PayloadPacket>, port: u16) {
    info!("handling advertisement packet");
    if let None =  id_to_packet.get(&advertisementpacket.packet) {
        debug!("Haven't received Payload Packet yet, sending send request");
        let sendrequest = SendablePackets::SendRequestPacket(SendRequestPacket::new(&advertisementpacket));
        socket.send_to(&encode(&sendrequest, SizeLimit::Infinite).unwrap(), (BROADCAST_ALL, port));
    } else {
        debug!("Already got advertised Packet, ignoring advertisement.");
    }
}

fn handle_send_request(sendrequestpacket: SendRequestPacket, socket: &UdpSocket, id_to_packet: &HashMap<PacketId, PayloadPacket>, ip_address: i64, port: u16) {
    info!("handling send request packet");
    if sendrequestpacket.favoured_sender == ip_address {
        debug!("send request is targeted at us, sending packet");
        if let Some(packet) = id_to_packet.get(&sendrequestpacket.packet) {
            socket.send_to(&encode(&SendablePackets::PayloadPacket(packet.clone()), SizeLimit::Infinite).unwrap(), (BROADCAST_ALL, port));
        }
    } else {
        debug!("send request is not targeted at us, ignoring");
    }
}

fn handle_payload(payloadpacket: PayloadPacket, socket: &UdpSocket, id_to_packet: &mut HashMap<PacketId, PayloadPacket>, ip_address: i64, port: u16, received: &Sender<PayloadPacket>) {
    info!("handling payload packet");
    if let None = id_to_packet.get(&payloadpacket.packet) {
        debug!("haven't gotten payload packet, saving");
        id_to_packet.insert(payloadpacket.packet.clone(), payloadpacket.clone());
        let advertisement = SendablePackets::AdvertisementPacket(AdvertisementPacket::new(&payloadpacket, ip_address));
        let advertisement_encoded = &encode(&advertisement, SizeLimit::Infinite).unwrap();
        info!("sending {} Bytes : {:?}", advertisement_encoded.len(), advertisement_encoded);
        socket.send_to(advertisement_encoded, (BROADCAST_ALL, port));

        if let Err(_) = received.send(payloadpacket.clone()) {
            warn!("Cannot forward received PayloadPacket");
        }
    } else {
        debug!("already received payload packet, ignoring");
    }
}

fn worker_loop(port: u16, ip_address: i64, socket: UdpSocket, rx: Receiver<PayloadPacket>, tx: Sender<PayloadPacket>) {
    info!("worker started!");
    let mut buffer = [0; 1500];
    let mut running = true;
    let mut id_to_payload = HashMap::new();

    while running {
        if let Ok(amount) = socket.recv_from(&mut buffer) {
            debug!("Received a message from the socket (length: {})", amount.0);
            //empty message queue
            let mut pending_message = PayloadPacket::new(&Vec::<u8>::with_capacity(0), 0, 0);
            while match rx.try_recv() {
                Ok(s) => {pending_message = s; true},
                Err(e) => {if e == TryRecvError::Disconnected { running = false; }; false}
                } != false {
                debug!("Got a pending payload package from the channel");
                let packet_id = pending_message.packet.clone();
                id_to_payload.insert(packet_id, pending_message.clone());
            }

            match decode(&buffer) {
                Err(e) => error!("Cannot decode recieved Packet. Error: {}", e),
                Ok(packet) => match packet {
                    SendablePackets::AdvertisementPacket(adv) => handle_advertisement(adv, &socket, &id_to_payload, port),
                    SendablePackets::SendRequestPacket(srp) => handle_send_request(srp, &socket, &id_to_payload, ip_address, port),
                    SendablePackets::PayloadPacket(pp) => handle_payload(pp, &socket, &mut id_to_payload, ip_address, port, &tx),
                }
            }
        }
    }
    debug!("worker thread exited");
}

