use std::collections::HashMap;
use crossbeam_channel::{select_biased, Receiver, RecvError, Sender};
use rustafarian_shared::assembler::assembler::Assembler;
use rustafarian_shared::assembler::disassembler::Disassembler;
use rustafarian_shared::messages::commander_messages::{SimControllerCommand, SimControllerResponseWrapper};
use rustafarian_shared::topology::Topology;
use wg_2024::config::Client;
use wg_2024::network::{NodeId, SourceRoutingHeader};
use wg_2024::packet::{FloodRequest, FloodResponse, Fragment, NodeType, Packet, PacketType};
use wg_2024::packet::PacketType::FloodRequest;

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

    topology: Topology,
    assembler: Assembler,
    disassembler: Disassembler,
}

impl ChatServer {

    pub fn new(
        id: NodeId,
        sim_controller_recv: Receiver<SimControllerCommand>, // SIM -> SER
        sim_controller_send: Sender<SimControllerResponseWrapper>, // SER -> SIM
        server_recv: Receiver<Packet>, // DRONES -> SER
        node_senders: HashMap<NodeId, Sender<Packet>>, // SER -> DRONES
    ) -> Self {
        ChatServer{
            id,
            sim_controller_recv,
            sim_controller_send,
            server_recv,
            node_senders,
            registered_clients: HashMap::new(),
            topology: Topology::new(),
            assembler: Assembler::new(),
            disassembler: Disassembler::new()
        }
    }

    /// Handle incoming commands from simulation controller
    fn handle_controller_commands(&mut self, command: Result<SimControllerCommand, RecvError>) {

    }

    /// Handle incoming messages
    fn handle_received_packet(&mut self, packet: Result<Packet, RecvError>) {

        // Handle error while receiving packet
        if packet.is_err() {
            eprintln!(
                "Error in the reception of the packet for the chat server with id {} - packet error: {}",
                self.id,
                packet.err().unwrap()
            );
            return;
        }

        let mut packet = packet.unwrap();
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
        let sender_node = flood_request.path_trace.last().unwrap();
        flood_request.increment(self.id, NodeType::Server);

        // Create a new packet and forward it to all the neighbours, except the sender node
        let new_flood_request = Packet::new_flood_request(
            SourceRoutingHeader::empty_route(),
            packet.session_id,
            flood_request
        );

        for (id, sender) in self.node_senders {
            if id != sender_node.0 {
                sender.send(new_flood_request.clone()).unwrap()
            }
        }

        // CASE 2 - Terminate the flood, create a flood response
        // let sender_node = flood_request.path_trace.last().unwrap();
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
        // self.node_senders.get(sender_node).unwrap().send(flood_response).unwrap();
    }

    /// Method that handles the reception of a flood response message, updating the topology.
    ///
    /// # Args
    /// * `flood_response: FloodResponse`: the flood response received by the server
    fn handle_flood_response(&mut self, flood_response: FloodResponse) {

        for (i, node) in flood_response.path_trace.iter().enumerate() {

            // Check if node is already in the topology, if not add it
            if !self.topology.nodes().contains(&node.0) {
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

                    self.topology.add_edge(node.0, previous_node.0);
                    self.topology.add_edge(previous_node.0, node.0);
                }
            }
        }
    }

    /// Run the server, listening for incoming commands from the simulation controller or incoming
    /// packets from the adjacent drones
    fn run(&mut self) {
        loop {
            select_biased! {

                // Handle commands from the simulation controller
                recv(self.sim_controller_recv) -> command => {

                }

                // Handle packets from adjacent drones
                recv(self.server_recv) -> packet => {

                }
            }
        }
    }
}