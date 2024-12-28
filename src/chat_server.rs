use std::collections::HashMap;
use crossbeam_channel::{select_biased, Receiver, RecvError, Sender};
use rustafarian_shared::assembler::assembler::Assembler;
use rustafarian_shared::assembler::disassembler::Disassembler;
use rustafarian_shared::messages::commander_messages::{SimControllerCommand, SimControllerEvent, SimControllerResponseWrapper};
use rustafarian_shared::topology::Topology;
use wg_2024::config::Client;
use wg_2024::network::{NodeId, SourceRoutingHeader};
use wg_2024::packet::{FloodRequest, FloodResponse, Fragment, NodeType, Packet, PacketType};
use crate::chat_server::LogLevel::{DEBUG, ERROR, INFO};

/// Used in the log method
/// * `INFO`: default log level, will always be printed
/// * `DEBUG`: used only in debug situation, will not print if the debug flag is `false`
/// * `ERROR`: will print the message to `io::stderr`
enum LogLevel { INFO, DEBUG, ERROR }

#[derive()]
/// Server used to send messages between clients
pub struct ChatServer {
    id: NodeId,

    // CROSSBEAM CHANNELS
    // Receiver where the sim controller will send commands. SIM -> SER
    sim_controller_recv: Receiver<SimControllerCommand>,
    // Sender where the server sends a response to the sim controller. SER -> SIM
    sim_controller_send: Sender<SimControllerResponseWrapper>,
    // Receiver where the server listens for incoming messages. DRONES -> SER
    server_recv: Receiver<Packet>,
    // Senders of the drones connected to the server. SER -> DRONES
    node_senders: HashMap<NodeId, Sender<Packet>>,

    // List of registered clients
    registered_clients: HashMap<NodeId, Client>,

    // HashMap containing a session as a key, and a vector of the packet sent for that session
    fragment_sent: HashMap<u64, Vec<Packet>>,

    topology: Topology,
    assembler: Assembler,
    disassembler: Disassembler,
    debug: bool // Debug flag, if true print verbose info
}

impl ChatServer {

    pub fn new(
        id: NodeId,
        sim_controller_recv: Receiver<SimControllerCommand>, // SIM -> SER
        sim_controller_send: Sender<SimControllerResponseWrapper>, // SER -> SIM
        server_recv: Receiver<Packet>, // DRONES -> SER
        node_senders: HashMap<NodeId, Sender<Packet>>, // SER -> DRONES
        debug: bool
    ) -> Self {
        ChatServer{
            id,
            sim_controller_recv,
            sim_controller_send,
            server_recv,
            node_senders,
            registered_clients: HashMap::new(),
            fragment_sent: HashMap::new(),
            topology: Topology::new(),
            assembler: Assembler::new(),
            disassembler: Disassembler::new(),
            debug
        }
    }

    /// Handle incoming commands from simulation controller
    pub fn handle_controller_commands(&mut self, command: Result<SimControllerCommand, RecvError>) {

    }

    /// Handle incoming messages
    pub fn handle_received_packet(&mut self, packet: Result<Packet, RecvError>) {

        // Handle error while receiving packet
        if packet.is_err() {
            let log_msg = format!(
                "Error in the reception of the packet for the chat server with id {} - packet error: {}",
                self.id,
                packet.err().unwrap()
            );
            self.log(log_msg.as_str(), ERROR);
            return;
        }

        let packet = packet.unwrap();
        self.log(
            format!("<- Received new packet of type {}", packet.pack_type).as_str(),
            DEBUG
        );

        match packet.pack_type.clone() {

            PacketType::MsgFragment(fragment) => {
                self.handle_message_fragment(packet, fragment)
            }
            PacketType::Ack(_) => {}
            PacketType::Nack(_) => {}
            PacketType::FloodRequest(flood_request) => {
                self.handle_flood_request(packet, flood_request);
            }
            PacketType::FloodResponse(flood_response) => {
                self.handle_flood_response(flood_response);
            }
        }
    }


    fn handle_message_fragment(&mut self, packet: Packet, fragment: Fragment) {

        // Reassemble message fragment, if a complete message is formed handle it
        if let Some(message) = self.assembler.add_fragment(fragment, packet.session_id) {
        }
        // TODO Send ACK for fragment
    }

    /// Method that handles the reception of a flood_request, currently update the path trace
    /// and forwards it to the other neighbours
    ///
    /// # Args
    /// * `packet: Packet`: the original packet received by the server, used to get information
    /// like session_id
    /// * `flood_request: FloodRequest`: received flood_request, will be updated and forwarded to
    /// neighbours
    fn handle_flood_request(&mut self, packet: Packet, mut flood_request: FloodRequest) {

        // CASE 1 - Forward the flood request
        // Get the node that sent the request, so to avoid sending the request back to it
        let sender_id = flood_request.path_trace.last().unwrap().0;
        self.log(format!("<- Flood request received from node {}", sender_id).as_str(), INFO);
        flood_request.increment(self.id, NodeType::Server);

        // Create a new packet and forward it to all the neighbours, except the sender node
        let new_flood_request = Packet::new_flood_request(
            SourceRoutingHeader::empty_route(),
            packet.session_id,
            flood_request
        );

        for (id, sender) in &self.node_senders {
            if *id != sender_id {
                match sender.send(new_flood_request.clone()) {

                    Ok(_) => {
                        self.log(
                            format!("-> Flood request correctly forwarded to node {}", id).as_str(),
                            DEBUG
                        );
                    }
                    Err(e) => {
                        self.log(
                            format!("An error occurred while sending a flood request - [{}]", e).as_str(),
                            ERROR
                        );
                    }
                };
            }
        }

        // CASE 2 - Terminate the flood, create a flood response
        // let sender_id = flood_request.path_trace.last().unwrap().0;
        // self.log(format!("<- Flood request received from node {}", sender_id).as_str(), INFO);
        // flood_request.increment(self.id, NodeType::Server);
        //
        // // Get the reverse path, from the server to the initiator
        // let mut route: Vec<u8> = flood_request.path_trace.iter().map(|node| node.0).collect();
        // route.reverse();
        //
        // let flood_response = Packet::new_flood_response(
        //     SourceRoutingHeader::new(route, 1),
        //     packet.session_id,
        //     FloodResponse {
        //         flood_id: flood_request.flood_id,
        //         path_trace: flood_request.path_trace
        //     }
        // );
        //
        // // Send it back to the node that sent the flood_request
        // self.log(
        //     format!("-> Flood response sent to node {}", sender_id).as_str(),
        //     DEBUG
        // );
        // self.node_senders.get(&sender_id).unwrap().send(flood_response).unwrap();
    }

    /// Method that handles the reception of a flood response message, updating the topology.
    ///
    /// # Args
    /// * `flood_response: FloodResponse`: the flood response received by the server
    fn handle_flood_response(&mut self, flood_response: FloodResponse) {

        self.log("<- New flood response received, updating topology", INFO);

        for (i, node) in flood_response.path_trace.iter().enumerate() {

            // Check if node is already in the topology, if not add it
            if !self.topology.nodes().contains(&node.0) {
                self.log(format!("New node ({}) added to the list", node.0).as_str(), DEBUG);
                self.topology.add_node(node.0);
            }

            // Skip the first node to avoid edge cases
            if i > 0 {

                let previous_node = flood_response.path_trace[i - 1];
                // Check if there is already an edge connecting the current node with
                // the previous one, if not add the connection, both from this node to the previous
                // one, and from the previous one to this.
                if !self.topology.edges()
                    .get(&node.0)
                    .unwrap()
                    .contains(&previous_node.0) {

                    self.log(
                        format!(
                            "Adding new edge between nodes {} and {}",
                            node.0,
                            previous_node.0
                        ).as_str(),
                        DEBUG
                    );
                    self.topology.add_edge(node.0, previous_node.0);
                    self.topology.add_edge(previous_node.0, node.0);
                }
            }
        }
    }


    /// Utility method used to cleanly log information, differentiating on three different levels
    ///
    /// # Args
    /// * `log_message: &str`: the message to log
    /// * `log_level: LogLevel`: the level of the log:
    ///     * `INFO`: default log level, will always be printed
    ///     * `DEBUG`: used only in debug situation, will not print if the debug flag is `false`
    ///     * `ERROR`: will print the message to `io::stderr`
    fn log(&self, log_message: &str, log_level: LogLevel) {

        match log_level {
            INFO => {
                print!("LEVEL: INFO >>> [Chat Server {}] - ", self.id);
                println!("{}", log_message);
            }
            DEBUG => {
                if self.debug {
                    print!("LEVEL: DEBUG >>> [Chat Server {}] - ", self.id);
                    println!("{}", log_message);
                }
            }
            ERROR => {
                eprint!("LEVEL: ERROR >>> [Chat Server {}] - ", self.id);
                eprintln!("{}", log_message);
            }
        }
    }


    /// Method that sends a packet
    ///
    /// # Args
    /// * `packet: Packet`: the packet to send along the network
    fn send_packet(&mut self, packet: Packet) {

        self.log(
            format!("-> Sending a new packet with session {}", packet.session_id).as_str(),
            INFO
        );
        // If the packet is a MsgFragment, then add it to the list of sent fragment
        let packet_type = packet.pack_type.clone();
        let session_id = packet.session_id.clone();

        if let PacketType::MsgFragment(_) =  packet_type.clone() {
            self.fragment_sent.entry(session_id)
                .or_insert_with(Vec::new)
                .push(packet.clone());
        }

        // Send the packet to the first node in the route, and handle different outcomes
        let node_id = packet.routing_header.hops[1];

        match self.node_senders.get(&node_id) {

            Some(node) => {
                match node.send(packet) {

                    Ok(_) => {
                        self.log(
                            format!(
                                "-> Packet with session {} sent correctly to node {}",
                                session_id,
                                node_id
                            ).as_str(),
                            DEBUG
                        );

                        // If packet sent correctly notify Simulation Controller
                        self.sim_controller_send.send(
                            SimControllerResponseWrapper::Event(
                                SimControllerEvent::PacketSent {
                                    session_id,
                                    packet_type: packet_type.to_string()
                                }
                            )
                        ).unwrap()
                    }

                    Err(e) => {
                        self.log(
                            format!(
                                "An error occurred while sending a packet - [Session Id: {}, Error: {}]",
                                session_id,
                                e
                            ).as_str(),
                            ERROR
                        );
                    }
                }
            }

            None => { self.log(format!("No node found with id {}", node_id).as_str(), ERROR); }
        }
    }

    pub fn topology(&mut self) -> &Topology { &self.topology }

    /// Run the server, listening for incoming commands from the simulation controller or incoming
    /// packets from the adjacent drones
    fn run(&mut self) {
        loop {
            select_biased! {
                // Handle commands from the simulation controller
                recv(self.sim_controller_recv) -> command => {
                    self.handle_controller_commands(command)
                }

                // Handle packets from adjacent drones
                recv(self.server_recv) -> packet => {
                    self.handle_received_packet(packet);
                }
            }
        }
    }
}