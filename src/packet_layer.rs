//use self::bincode::SizeLimit;

use std::clone::Clone;
use std::collections::{HashMap, VecDeque};
use std::marker::Send;
use std::sync::mpsc::{channel, Sender, Receiver};
use std::thread;
use std::thread::{JoinHandle};
use bincode::{serialize, deserialize, Infinite};
use serde::{Serialize, Deserialize};
use serde::de::DeserializeOwned;
use std::sync::mpsc::{TryRecvError, RecvError};
use std::sync::Arc;
use std::net::{UdpSocket, SocketAddr};
use std::io::Error as IOError;

static IP_ADDR_ANY : &'static str = "0.0.0.0";
static BROADCAST_ALL : &'static str = "255.255.255.255";
const MAX_PACKETS_STORED: usize = 200;


#[derive(Deserialize, Serialize, PartialEq, Eq, Clone, Hash)]
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

#[derive(Deserialize, Serialize, PartialEq, Eq, Clone, Hash)]
pub struct AdvertisementPacket {
    packet: PacketId,
    advertiser: i64,
}

impl AdvertisementPacket {
    pub fn new<P>(packet: &PayloadPacket<P>, advertiser: i64) -> Self {
        AdvertisementPacket {
            packet: packet.packet.clone(),
            advertiser: advertiser,
        }
    }
}

#[derive(Deserialize, Serialize, PartialEq, Eq, Clone, Hash)]
pub struct SendRequestPacket {
    packet: PacketId,
}

impl SendRequestPacket {
    pub fn new(packet: &AdvertisementPacket) -> Self {
        SendRequestPacket {
            packet: packet.packet.clone(),
        }
    }
}

#[derive(Deserialize, Serialize, PartialEq, Eq, Clone, Hash)]
pub struct PayloadPacket<P> {
    packet: PacketId,
    payload: P,
}

impl<P> PayloadPacket<P> {
    pub fn new(payload: P, source_ip_addr: i64, sequence_number: u8) -> Self {
        PayloadPacket {
            packet: PacketId::new(source_ip_addr, sequence_number),
            payload: payload,
        }
    }
}

#[derive(Deserialize, Serialize, PartialEq)]
enum SendablePackets<P> {
    AdvertisementPacket(AdvertisementPacket),
    SendRequestPacket(SendRequestPacket),
    PayloadPacket(PayloadPacket<P>),
}

pub struct PacketSender<P> {
    ip_address: i64,
    port: u16,
    sequence_number: u8,
    sender: Sender<PayloadPacket<P>>,
    socket: UdpSocket,
    worker: Arc<JoinHandle<()>>,
}

impl<P> PacketSender<P> 
	where P: Serialize {
    pub fn send(&mut self, payload: P) {
        debug!("got new payload to send");
        let packet = PayloadPacket::new(payload, self.ip_address, self.sequence_number);
        let advertisement : SendablePackets<P> = SendablePackets::AdvertisementPacket(AdvertisementPacket::new(&packet, self.ip_address));
        self.sender.send(packet);

        let advertisement_encoded = &serialize(&advertisement, Infinite).unwrap();
        debug!("sending {} Bytes", advertisement_encoded.len());
        match self.socket.send_to(&advertisement_encoded, (BROADCAST_ALL, self.port)) {
            Ok(_) => debug!("Successfully sent advertisement!"),
            Err(e) => error!("Failed to send advertisement: {}", e)
        }
        self.sequence_number = self.sequence_number.wrapping_add(1);
        debug!("incremented sequence_number to {}", self.sequence_number);
    }
}

pub struct PacketReceiver<P> {
    receiver: Receiver<PayloadPacket<P>>,
    worker: Arc<JoinHandle<()>>,
}

impl<P> PacketReceiver<P> {
    pub fn receive(&self) -> Result<(P, i64, u8), RecvError> {
        let packet = self.receiver.recv()?;
        Ok((packet.payload, packet.packet.source_ip_addr, packet.packet.sequence_number))
    }

    pub fn try_receive(&self) -> Result<(P, i64, u8), TryRecvError> {
        let packet = self.receiver.try_recv()?;
        Ok((packet.payload, packet.packet.source_ip_addr, packet.packet.sequence_number))
    }
}

pub fn packet_layer<P: 'static>(port: u16, ip_address:i64) -> Result<(PacketSender<P>, PacketReceiver<P>), IOError> 
	where for<'de> P: Send + Clone + Serialize + Deserialize<'de>{

    let (send_tx, send_rx) = channel();
    let (receive_tx, receive_rx) = channel();

    let socket = (UdpSocket::bind((IP_ADDR_ANY, port)))?;
    let worker_socket = socket.try_clone().unwrap();
    worker_socket.set_broadcast(true)?;

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

fn handle_advertisement<P>(advertisementpacket: AdvertisementPacket, socket: &UdpSocket, id_to_packet: &HashMap<PacketId, PayloadPacket<P>>, source: SocketAddr) 
	where P:Serialize {
    info!("handling advertisement packet");
    if let None =  id_to_packet.get(&advertisementpacket.packet) {
        debug!("Haven't received Payload Packet yet, sending send request");
        let sendrequest : SendablePackets<P> = SendablePackets::SendRequestPacket(SendRequestPacket::new(&advertisementpacket));
        socket.send_to(&serialize(&sendrequest, Infinite).unwrap(), source).unwrap_or_else(|err| {error!("Failed to send advertisement, got {}", err); 0});
    } else {
        debug!("Already got advertised Packet, ignoring advertisement.");
    }
}

fn handle_send_request<P>(sendrequestpacket: SendRequestPacket, socket: &UdpSocket, id_to_packet: &HashMap<PacketId, PayloadPacket<P>>, source: SocketAddr) 
	where P: Clone + Serialize{
    info!("handling send request packet");
    if let Some(packet) = id_to_packet.get(&sendrequestpacket.packet) {
        socket.send_to(&serialize(&SendablePackets::PayloadPacket(packet.clone()), Infinite).unwrap(), source).unwrap_or_else(|err| {error!("Failed to send send request, got {}", err); 0});
    } else {
        debug!("failed to find requested packet, ignoring");
    }
}

fn handle_payload<P>(payloadpacket: PayloadPacket<P>, socket: &UdpSocket, id_to_packet: &mut HashMap<PacketId, PayloadPacket<P>>, id_age: &mut VecDeque<PacketId>, ip_address: i64, port: u16, received: &Sender<PayloadPacket<P>>) 
	where P: Clone + Serialize {
    info!("handling payload packet");
    if let None = id_to_packet.get(&payloadpacket.packet) {
        debug!("haven't gotten payload packet, saving");
        
        id_to_packet.insert(payloadpacket.packet.clone(), payloadpacket.clone());
		id_age.push_back(payloadpacket.packet.clone());
		if id_age.len() > MAX_PACKETS_STORED {
			id_to_packet.remove(&id_age.pop_front().unwrap());
		}

        let advertisement : SendablePackets<P> = SendablePackets::AdvertisementPacket(AdvertisementPacket::new(&payloadpacket, ip_address));
        let advertisement_encoded = &serialize(&advertisement, Infinite).unwrap();
        info!("sending {} Bytes : {:?}", advertisement_encoded.len(), advertisement_encoded);
        socket.send_to(advertisement_encoded, (BROADCAST_ALL, port)).unwrap_or_else(|err| {error!("Failed to send payload, got {}", err); 0});

        if let Err(_) = received.send(payloadpacket.clone()) {
            warn!("Cannot forward received PayloadPacket");
        }
    } else {
        debug!("already received payload packet, ignoring");
    }
}

fn worker_loop<P>(port: u16, ip_address: i64, socket: UdpSocket, rx: Receiver<PayloadPacket<P>>, tx: Sender<PayloadPacket<P>>) 
	where P: Clone + DeserializeOwned + Serialize{
    info!("worker started!");
    let mut buffer = [0; 4*10240];
    let mut running = true;
    let mut id_to_payload = HashMap::new();
    let mut id_age = VecDeque::with_capacity(MAX_PACKETS_STORED);

    while running {
        if let Ok((amount, source)) = socket.recv_from(&mut buffer) {
            debug!("Received a message from the socket (length: {})", amount);
            //empty message queue
            
            while let Ok(pending_message) = rx.try_recv() {
                debug!("Got a pending payload package from the channel");
                let packet_id = pending_message.packet.clone();
                id_to_payload.insert(packet_id, pending_message);
            }
            if let Err(TryRecvError::Disconnected) = rx.try_recv(){
            	running = false;
            }

            match deserialize(&buffer) {
                Err(e) => error!("Cannot decode recieved Packet. Error: {}", e),
                Ok(packet) => match packet {
                    SendablePackets::AdvertisementPacket(adv) => handle_advertisement(adv, &socket, &id_to_payload, source),
                    SendablePackets::SendRequestPacket(srp) => handle_send_request(srp, &socket, &id_to_payload, source),
                    SendablePackets::PayloadPacket(pp) => handle_payload(pp, &socket, &mut id_to_payload, &mut id_age, ip_address, port, &tx),
                }
            }
        }
    }
    debug!("worker thread exited");
}

