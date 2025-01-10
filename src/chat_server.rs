use std::collections::{HashMap, HashSet};
use chrono::{Utc};
use crossbeam_channel::{select_biased, Receiver, RecvError, Sender};
use rand::Rng;
use rustafarian_shared::assembler::assembler::Assembler;
use rustafarian_shared::assembler::disassembler::Disassembler;
use rustafarian_shared::messages::commander_messages::{SimControllerCommand, SimControllerEvent, SimControllerMessage, SimControllerResponseWrapper};
use rustafarian_shared::topology::{Topology};
use rustafarian_shared::logger::{Logger};
use rustafarian_shared::logger::LogLevel::{ERROR, INFO, DEBUG};
use wg_2024::network::{NodeId, SourceRoutingHeader};
use wg_2024::packet::{Ack, FloodRequest, FloodResponse, Fragment, Nack, NackType, NodeType, Packet, PacketType};
use rustafarian_shared::messages::chat_messages::{ChatRequest, ChatRequestWrapper, ChatResponse, ChatResponseWrapper};
use rustafarian_shared::messages::general_messages::{DroneSend, ServerType, ServerTypeResponse};
use rustafarian_shared::TIMEOUT_BETWEEN_FLOODS_MS;
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
    registered_clients: HashSet<NodeId>,

    // HashMap containing a session as a key, and a vector of the packet sent for that session
    fragment_sent: HashMap<u64, Vec<Packet>>,
    // Set containing tuple of (session_id, fragment_id) that the server must retry to send
    fragment_retry_queue: HashSet<(u64, u64)>,

    topology: Topology,
    assembler: Assembler,
    disassembler: Disassembler,
    current_flood_id: u64,

    // Timestamp when the last flood has begun, used to understand if the server can start a new one
    // based on the shared timeout
    last_flood_timestamp: i64,
    logger: Logger
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
            registered_clients: HashSet::new(),
            fragment_sent: HashMap::new(),
            fragment_retry_queue: HashSet::new(),
            topology: Topology::new(),
            assembler: Assembler::new(),
            disassembler: Disassembler::new(),
            current_flood_id: 0,
            last_flood_timestamp: 0,
            logger: Logger::new("Chat Server".to_string(), id, debug)
        }
    }

    /// Handle incoming commands from simulation controller
    pub fn handle_controller_commands(&mut self, command: Result<SimControllerCommand, RecvError>) {

        // Handle error while receiving command
        if command.is_err() {
            let log_msg = format!(
                "Error in the reception of a command from Simulation Controller - error: [{}]",
                command.err().unwrap()
            );
            self.logger.log(log_msg.as_str(), ERROR);
            return;
        }

        self.logger.log("<- Received new command from Simulation Controller", INFO);

        let command = command.unwrap();
        match command {
            SimControllerCommand::AddSender(node_id, sender) => {

                self.logger.log("Received AddSender command from SC", DEBUG);
                self.handle_add_sender_command(node_id, sender);
            }
            SimControllerCommand::RemoveSender(node_id) => {

                self.logger.log("Received AddSender command from SC", DEBUG);
                self.handle_remove_sender_command(node_id);
            }
            SimControllerCommand::Topology => {

                self.logger.log("Received Topology command from SC", DEBUG);
                self.handle_topology_command()
            }

            _ => {
                self.logger.log(
                    format!(
                        "Received an unexpected command from Simulation Controller, skip handling.\
                         Command: [{:?}]",
                        command
                    ).as_str(),
                    ERROR
                );
            }
        }
    }

    /// Handle incoming messages
    pub fn handle_received_packet(&mut self, packet: Result<Packet, RecvError>) {

        // Handle error while receiving packet
        if packet.is_err() {
            let log_msg = format!(
                "Error in the reception of a packet - packet error: [{}]",
                packet.err().unwrap()
            );
            self.logger.log(log_msg.as_str(), ERROR);
            return;
        }

        let packet = packet.unwrap();
        self.logger.log(
            format!("<- Received new packet of type [{}]", packet.pack_type).as_str(),
            INFO
        );

        match packet.pack_type.clone() {

            PacketType::MsgFragment(fragment) => {
                self.logger.log("Received MsgFragment", DEBUG);
                self.handle_message_fragment(packet, fragment)
            }
            PacketType::Ack(ack) => {
                self.logger.log("Received ACK", DEBUG);
                self.handle_ack(packet, ack);
            }
            PacketType::Nack(nack) => {
                self.logger.log("Received NACK", DEBUG);
                self.handle_nack(packet, nack)
            }
            PacketType::FloodRequest(flood_request) => {
                self.logger.log("Received FloodRequest", DEBUG);
                self.handle_flood_request(packet, flood_request);
            }
            PacketType::FloodResponse(flood_response) => {
                self.logger.log("Received FloodResponse", DEBUG);
                self.handle_flood_response(packet, flood_response);
            }
        }
    }


    // <== DRONE PACKET HANDLING ==>

    /// Method that handles a message fragments, try and form a complete message and sends an ACK
    /// back to the sender
    ///
    /// # Args
    /// * `packet: Packet` - the packet containing the fragment, used to gather information like
    /// the sender id
    /// * `fragment: Fragment` - the actual fragment, used to reconstruct a complete message
    fn handle_message_fragment(&mut self, packet: Packet, fragment: Fragment) {

        let session_id = packet.session_id.clone();
        let fragment_id = fragment.fragment_index.clone();
        let sender_id = packet.routing_header.hops[0];

        self.logger.log(
            format!(
                "<- New message fragment received - [Node: {}, Session: {}, Fragment: {}]",
                sender_id,
                session_id,
                fragment_id
            ).as_str(),
            INFO
        );

        // Send ACK to the sender
        let mut fallback_route = packet.routing_header.hops;
        fallback_route.reverse();
        self.send_ack(sender_id, fragment_id, session_id, fallback_route);

        // Reassemble message fragment, if a complete message is formed handle it
        if let Some(message) = self.assembler.add_fragment(fragment, session_id) {

            let message = String::from_utf8_lossy(&message).to_string();
            self.logger.log(format!("A complete message was formed: [{}]", message).as_str(), DEBUG);
            self.handle_complete_message(message, session_id, sender_id);
        }
    }

    /// Handle an ACK, removing the corresponding fragment from the list of fragment sent
    ///
    /// # Args
    /// * `packet: Packet` - the packet containing the ACK, used to gather information like
    /// the sender id
    /// * `ack: Ack` - the ACK containing the fragment id
    fn handle_ack(&mut self, packet: Packet, ack: Ack) {

        let session_id = packet.session_id.clone();
        let fragment_id = ack.fragment_index;
        let sender_id = packet.routing_header.hops[0];

        self.logger.log(format!("Received ACK from node [{}]", sender_id).as_str(), INFO);

        // If there are fragments for the current session
        if let Some(fragments) = self.fragment_sent.get_mut(&session_id) {

            // If there is a fragment for the current session with the right fragment id,
            // then remove it
            fragments.retain(|packet| {
                match &packet.pack_type {
                    PacketType::MsgFragment(fragment) => {
                        fragment.fragment_index != fragment_id
                    }
                    _ => true
                }
            });

            // Remove the session id from the list of fragment if all fragment have been correctly
            // received
            if fragments.is_empty() { self.fragment_sent.remove(&session_id); }
        } else {
            // NO SESSION FOUND
            self.logger.log(format!("No session with id [{}] is registered", session_id).as_str(), DEBUG);
        }
    }

    /// Handle a NACK, adding the nacked fragment to a list in order to retry later
    ///
    /// # Args
    /// * `packet: Packet` - the packet containing the NACK, used to gather information like
    /// the sender id
    /// * `nack: Nacl` - the NACK containing the fragment id
    fn handle_nack(&mut self,  packet: Packet, nack: Nack) {

        let session_id = packet.session_id.clone();
        let fragment_id = nack.fragment_index;
        let nack_type = nack.nack_type;
        let sender_id = packet.routing_header.hops[0];

        self.logger.log(
            format!("Received NACK from node [{}], nack type [{:?}]", sender_id, nack_type)
                .as_str(),
            INFO
        );

        // Avoid starting a new flood if the packet has been dropped,
        if nack_type == NackType::Dropped {
            self.topology.update_node_history(&vec![sender_id], true);
            self.send_packet(packet);
            return;
        }

        // Add tuple to list in order to retry and send the fragment
        self.fragment_retry_queue.insert((session_id, fragment_id));

        // If no flood is currently in process, start a new one to update the topology
        self.start_flooding()
    }

    /// Method that handles the reception of a flood_request, currently update the path trace
    /// and forwards it to the other neighbours
    ///
    /// # Args
    /// * `packet: Packet` - the original packet received by the server, used to get information
    /// like session_id
    /// * `flood_request: FloodRequest` - received flood_request, will be updated and forwarded to
    /// neighbours
    fn handle_flood_request(&mut self, packet: Packet, mut flood_request: FloodRequest) {

        // CASE 1 - Forward the flood request
        // Get the node that sent the request, so to avoid sending the request back to it
        let sender_id = flood_request.path_trace.last().unwrap().0;
        self.logger.log(format!("<- Flood request received from node [{}]", sender_id).as_str(), INFO);
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
                        self.logger.log(
                            format!("-> Flood request correctly forwarded to node [{}]", id).as_str(),
                            DEBUG
                        );
                    }
                    Err(e) => {
                        self.logger.log(
                            format!("An error occurred while sending a flood request - [{}]", e).as_str(),
                            ERROR
                        );
                    }
                };
            }
        }

        // CASE 2 - Terminate the flood, create a flood response
        // let sender_id = flood_request.path_trace.last().unwrap().0;
        // self.logger.log(format!("<- Flood request received from node [{}]", sender_id).as_str(), INFO);
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
        // self.logger.log(
        //     format!("-> Flood response sent to node [{}]", sender_id).as_str(),
        //     DEBUG
        // );
        // self.node_senders.get(&sender_id).unwrap().send(flood_response).unwrap();
    }

    /// Method that handles the reception of a flood response message, updating the topology, or
    /// forwarding it to the correct node.
    ///
    /// # Args
    /// * `packet: Packet` - the original packet received by the server, used to forward the flood
    /// response in case it does not belong to this server
    /// * `flood_response: FloodResponse` - the flood response received by the server
    fn handle_flood_response(&mut self, mut packet: Packet, flood_response: FloodResponse) {

        // If the flood was not started by this server, then forward it
        let initiator = flood_response.path_trace[0].0;
        if initiator != self.id {

            self.logger.log(
                "<- Received a flood response, but the server is not the initiator.\
                Updating the topology and sending it along the network",
                INFO
            );

            // Check if the server is the right receiver, if so advance the hop index and send it
            let current_node = packet.routing_header.current_hop().unwrap();
            if current_node == self.id {

                packet.routing_header.increase_hop_index();
                self.send_packet(packet);
            } else {
                // TODO Should never happen (I think...), check
                self.logger.log(
                    format!(
                        "Server is not the right recipient, expecting node {}",
                        current_node
                    ).as_str(),
                    ERROR
                );
            }
        } else {
            self.logger.log("<- New flood response received, updating topology", INFO);
        }

        if flood_response.flood_id != self.current_flood_id {
            self.logger.log(
                "<- Received a flood response with an id different from the current one",
                DEBUG
            );
        }

        for (i, node) in flood_response.path_trace.iter().enumerate() {

            // Check if node is already in the topology, if not add it
            if !self.topology.nodes().contains(&node.0) {
                self.logger.log(format!("New node [{}] added to the list", node.0).as_str(), DEBUG);
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

                    self.logger.log(
                        format!(
                            "Adding new edge between nodes [{}] and [{}]",
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

        // Resend fragment if any are present in the queue
        if !self.fragment_retry_queue.is_empty() { self.resend_fragments(); }
    }

    /// Handle request from the clients
    ///
    /// # Args
    /// * `message: String` - the message received from the client
    /// * `session_id: u64` - the session of the current message
    /// * `destination_node: NodeId` - the id of the client that sent the message, used to send the
    /// response to
    fn handle_complete_message(
        &mut self,
        message: String,
        session_id: u64,
        destination_node: NodeId
    ) {

        match ChatRequestWrapper::from_string(message) {

            Ok(req_wrapper) => {

                self.logger.log("Successfully deserialized the message", DEBUG);
                match req_wrapper {

                    ChatRequestWrapper::Chat(request) => {
                        
                        match request {

                            ChatRequest::ClientList => {
                                self.handle_client_list_request(destination_node, session_id);
                            }
                            ChatRequest::Register(node_id  ) => {

                                // Check if the node to register is the same that initiated the
                                // request, if not do nothing
                                if node_id != destination_node {
                                    self.logger.log(
                                        format!(
                                            "Initiator id [{}] is not equal to the node id to register [{}]",
                                            destination_node,
                                            node_id
                                        ).as_str(),
                                        ERROR
                                    );
                                    return;
                                }

                                self.handle_register_request(node_id, session_id);
                            }
                            ChatRequest::SendMessage {
                                from: sender_id,
                                to: receiver_id,
                                message
                            } => {
                                self.handle_send_message_request(
                                    sender_id,
                                    receiver_id,
                                    session_id,
                                    message
                                );
                            }
                        }
                    }
                    ChatRequestWrapper::ServerType(_) => {
                        self.handle_server_type_request(destination_node, session_id);
                    }
                }
            }
            Err(e) => {
                self.logger.log(
                    format!("Error while deserializing message from string [{}]", e).as_str(),
                    ERROR
                )
            }
        }
    }

    /// Send list of registered clients
    ///
    /// # Args
    /// * `node_id: NodeId` - id of the client that requested the list
    /// * `session_id: u64` - the session this message belongs to
    fn handle_client_list_request(&mut self, node_id: NodeId, session_id: u64) {

        self.logger.log(format!("<- Received request from [{}] to get client list", node_id).as_str(), INFO);

        let response = ChatResponseWrapper::Chat(
            ChatResponse::ClientList(self.registered_clients.clone().into_iter().collect())
        );
        self.send_message(response.stringify(), node_id, session_id);
    }

    /// Register a client to the server
    ///
    /// # Args
    /// * `node_id: NodeId` - id of the client to register
    /// * `session_id: u64` - the session this message belongs to
    fn handle_register_request(&mut self, node_id: NodeId, session_id: u64) {

        self.logger.log(format!("New client [{}] registered!", node_id).as_str(), INFO);
        self.registered_clients.insert(node_id);

        // Send response once client is registered
        let response = ChatResponseWrapper::Chat(
            ChatResponse::ClientRegistered
        );
        self.send_message(response.stringify(), node_id, session_id);
    }

    /// Send a message from the sender to the receiver
    ///
    /// # Args
    /// * `sender_id: NodeId` - id of the sending client
    /// * `receiver_id: NodeId` - id of the receiving client
    /// * `session_id: u64` - the session this message belongs to
    /// * `message: String` - the message to send
    fn handle_send_message_request(
        &mut self,
        sender_id: NodeId,
        receiver_id: NodeId,
        session_id: u64,
        message: String
    ) {

        self.logger.log(
            format!(
                "-> Sending a message from client [{}] to client [{}]",
                sender_id,
                receiver_id
            ).as_str(),
            INFO
        );

        if !self.registered_clients.contains(&receiver_id) {
            self.logger.log(
                format!(
                    "Cannot send message to client [{}], it needs to be registered",
                    receiver_id
                ).as_str(),
                ERROR
            );
            return;
        }

        // Sending message to receiver
        let msg_response = ChatResponseWrapper::Chat(
            ChatResponse::MessageFrom { from: sender_id, message: message.into_bytes() }
        );

        // Generate a new random session_id
        let mut rng = rand::thread_rng();
        let new_session_id = rng.gen();

        self.send_message(msg_response.stringify(), receiver_id, new_session_id);

        // Sending confirmation to the sender that the message has been sent
        self.logger.log(
            format!("-> Confirming message sent successfully to sender [{}]", sender_id).as_str(),
            DEBUG
        );
        let msg_response = ChatResponseWrapper::Chat(
            ChatResponse::MessageSent
        );
        self.send_message(msg_response.stringify(), sender_id, session_id);
    }

    /// Handle a request from a chat client to obtain this server type
    ///
    /// # Args
    /// * `destination_node: NodeId` - id of the client that made the request
    /// * `session_id: u64` - the session this message belongs to
    fn handle_server_type_request(&mut self, destination_node: NodeId, session_id: u64) {

        self.logger.log("Handling server type request", INFO);

        // Send response with server type
        let response = ChatResponseWrapper::ServerType(
            ServerTypeResponse::ServerType(ServerType::Chat)
        );
        self.send_message(response.stringify(), destination_node, session_id);
    }


    // <== SIMULATION CONTROLLER COMMAND HANDLING ==>

    /// Method that handles the AddSender command from the Simulation Controller, adding the node
    /// and sender to the list of neighbours and updating the topology
    ///
    /// # Args
    /// `node_id: NodeId` - the node to add
    /// `sender: Sender<Packet>` - the sender channel of the node to add
    fn handle_add_sender_command(&mut self, node_id: NodeId, sender: Sender<Packet>) {

        self.logger.log(format!("Adding node [{}] to neighbours", node_id).as_str(), INFO);

        // Check that the node is not present among neighbours
        if self.node_senders.contains_key(&node_id) {
            // TODO Should this be INFO or ERROR level?
            self.logger.log(format!("Node [{}], already found among neighbours", node_id).as_str(), ERROR);
            return;
        }

        self.node_senders.insert(node_id, sender);
        // Update the topology adding the current node, and an edge between the server and node
        self.update_topology(vec![node_id], vec![(self.id, node_id)]);

        self.start_flooding();
    }

    /// Method that handles the RemoveSender command from Simulation Controller, removing the node
    /// from the list of neighbours, and updating the topology accordingly
    ///
    /// # Args
    /// `node_id: NodeId` - the node to remove
    fn handle_remove_sender_command(&mut self, node_id: NodeId) {

        if self.node_senders.contains_key(&node_id) {
            self.node_senders.remove(&node_id);
            self.topology.remove_edges(self.id, node_id);

            self.logger.log(
                format!(
                    "Node with id [{}] successfully removed from neighbours",
                    node_id
                ).as_str(),
                INFO
            );
        } else {
            self.logger.log(
                format!(
                    "No node with id [{}] found among neighbours, doing nothing",
                    node_id
                ).as_str(),
                INFO
            );
        }
    }

    /// Method that handles the Topology command from Simulation Controller, sending the current
    /// topology
    fn handle_topology_command(&mut self) {

        self.logger.log("Sending current topology to Simulation Controller", INFO);

        let response = SimControllerResponseWrapper::Message(
            SimControllerMessage::TopologyResponse(self.topology.clone())
        );

        match self.sim_controller_send.send(response) {

            Ok(_) => {
                self.logger.log("Topology successfully delivered to Simulation Controller", DEBUG);
            }
            Err(e) => {
                self.logger.log(
                    format!(
                        "Error while sending topology to Simulation Controller - error [{:?}]",
                        e
                    ).as_str(),
                    ERROR
                );
            }
        }
    }


    // <== UTILITY METHODS ==>

    /// Send a new message to a destination, fragmenting it in smaller chunks using the disassembler
    ///
    /// # Args
    /// * `message: String` - the message to be sent, still to be fragmented
    /// * `destination_node: NodeId` - the destination to send the message to
    /// * `session_id: u64` - the session this message belongs to
    fn send_message(&mut self, message: String, destination_node: NodeId, session_id: u64) {

        self.logger.log(
            format!("-> Sending a new message to node [{}]", destination_node).as_str(),
            INFO
        );

        // If packet sent correctly notify Simulation Controller
        match self.sim_controller_send.send(
            SimControllerResponseWrapper::Event( SimControllerEvent::MessageSent {session_id} )) {

            Ok(_) => {
                self.logger.log("MessageSent event successfully delivered to Simulation Controller", DEBUG);
            }
            Err(e) => {
                self.logger.log(
                    format!(
                        "Error while sending MessageSent event to Simulation Controller - error [{:?}]",
                        e
                    ).as_str(),
                    ERROR
                );
            }
        }

        let fragments = self
            .disassembler
            .disassemble_message(message.as_bytes().to_vec(), session_id);

        // Find the route to the destination node (the client that sent the request) and create
        // the common header for all the fragments
        let header = self.topology.get_routing_header(self.id, destination_node);

        // If no route was found, then add the fragments to resend later, and start a new flood
        if header.is_empty() {
            self.logger.log(
                format!(
                    "Trying to send a message to node [{}], but no route was found, starting new flood",
                    destination_node
                ).as_str(),
                ERROR
            );


            for fragment in fragments {

                // Add to retry queue
                self.fragment_retry_queue.insert((session_id, fragment.fragment_index));


                // Create a new header with only the destination_node in the route, this way
                // when resending the packet the server knows the destination
                let header = SourceRoutingHeader::new(
                    vec![destination_node],
                    0
                );

                // Create a packet for each fragment and save it in the server memory, used when
                // the fragment will be re-handled later
                let packet = Packet::new_fragment(header, session_id, fragment);
                self.fragment_sent.entry(session_id)
                    .or_insert_with(Vec::new)
                    .push(packet.clone());
            }
            self.start_flooding();
        } else {
            self.logger.log(
                format!(
                    "New header created - [Destination: {}, Route: {:?}]",
                    destination_node,
                    header.hops
                ).as_str(),
                DEBUG
            );

            // Send each fragment to the node, using send_packet method
            for fragment in fragments {

                let packet = Packet::new_fragment(header.clone(), session_id, fragment);
                self.send_packet(packet);
            }
        }
    }

    /// Method that sends a packet
    ///
    /// # Args
    /// * `packet: Packet` - the packet to send along the network
    fn send_packet(&mut self, packet: Packet) {

        self.logger.log(
            format!("-> Sending a new packet with session [{}]", packet.session_id).as_str(),
            INFO
        );
        // If the packet is a MsgFragment, then add it to the list of sent fragment
        let packet_type = packet.pack_type.clone();
        let session_id = packet.session_id.clone();

        // If packet has empty route, abort
        if packet.routing_header.hops.is_empty() {
            self.logger.log("Packet has an empty route, abort sending!", ERROR);
            return
        }

        if let PacketType::MsgFragment(_) =  packet_type.clone() {
            self.fragment_sent.entry(session_id)
                .or_insert_with(Vec::new)
                .push(packet.clone());

            // Only update history if packet is MsgFragment
            self.topology.update_node_history(&packet.routing_header.hops, false);
        }

        // Send the packet to the next node in the route
        let node_id = packet.routing_header.current_hop().unwrap();
        match self.node_senders.get(&node_id) {

            Some(node) => {
                match node.send(packet) {

                    Ok(_) => {
                        self.logger.log(
                            format!(
                                "-> Packet with session [{}] sent correctly to node [{}]",
                                session_id,
                                node_id
                            ).as_str(),
                            INFO
                        );
                    }

                    Err(e) => {
                        self.logger.log(
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

            None => { self.logger.log(format!("No node found with id [{}]", node_id).as_str(), ERROR); }
        }
    }

    /// Method that sends an ACK for a given fragment in a given session, using the `send_packet`
    /// method, to a specific node.
    ///
    /// # Args
    /// * `destination_node: NodeId` - the selected destination node, used to compute the path
    /// * `fragment_id: u64` - the id of the fragment the ACK refers to
    /// * `session_id: u64` - the current session the fragment belongs to
    /// * `fallback_route: Vec<NodeId>` - used when the server is unable to compute a route
    fn send_ack(
        &mut self,
        destination_node: NodeId,
        fragment_id: u64,
        session_id: u64,
        fallback_route: Vec<NodeId>
    ) {

        self.logger.log(
            format!(
                "-> Sending an ACK to node [{}] for fragment [{}]",
                destination_node,
                fragment_id
            ).as_str(),
            INFO
        );

        // Create a new routing header and compute route
        let mut routing_header = self
            .topology
            .get_routing_header(self.id, destination_node);

        // If no route was computed, use the fallback one
        if routing_header.is_empty() {
            self.logger.log("No route computed, using fallback one", DEBUG);
            routing_header.hops = fallback_route;
        }

        self.logger.log(
            format!(
                "New header created - [Destination: {}, Route: {:?}]",
                destination_node,
                routing_header.hops
            ).as_str(),
            DEBUG
        );

        // Create a new packet and send it
        let packet = Packet::new_ack(routing_header, session_id, fragment_id);
        self.send_packet(packet);
    }

    /// Initiate a new flooding sequence, checking if the server is not waiting on a flood response
    fn start_flooding(&mut self) {

        // Check if server is waiting on an old flood
        if !self.can_flood() {
            self.logger.log("Server is already waiting on a flood", INFO);
            return;
        }

        self.logger.log("Starting a new flooding", INFO);

        // Send event to Simulation Controller that a new flood has begun
        match self
            .sim_controller_send
            .send(SimControllerResponseWrapper::Event(SimControllerEvent::FloodRequestSent)) {

            Ok(_) => {
                self.logger.log("FloodRequestSent event successfully delivered to Simulation Controller", DEBUG);
            }
            Err(e) => {
                self.logger.log(
                    format!(
                        "Error while sending FloodRequestSent event to Simulation Controller - error [{:?}]",
                        e
                    ).as_str(),
                    ERROR
                );
            }
        }

        let mut rng = rand::thread_rng();
        // Generate new random flood_id and set it as the current id in the server
        self.current_flood_id = rng.gen();

        // Create a flood_request to be sent to all the neighbours
        let session_id: u64 = rng.gen();
        let flood_request = Packet::new_flood_request(
            SourceRoutingHeader::empty_route(),
            session_id,
            FloodRequest::initialize(self.current_flood_id, self.id, NodeType::Server)
        );

        for (id, sender) in self.node_senders.clone() {
            match sender.send(flood_request.clone()) {
                Ok(_) => {
                    self.logger.log(
                      format!("Flood request correctly sent to node [{}]", id).as_str(),
                      DEBUG
                    );

                    self.last_flood_timestamp = Utc::now().timestamp_millis();
                }
                Err(e) => {
                    self.logger.log(
                        format!(
                            "Error while sending flood request to node [{}] - error [{}]",
                            id,
                            e
                        ).as_str(),
                        ERROR
                    );
                }
            }
        }
    }

    /// Method that try to resend each fragment in the fragment_retry_queue
    fn resend_fragments(&mut self) {

        self.logger.log("Resending fragments in the fragment_retry_queue", DEBUG);

        for (session_id, fragment_id) in self.fragment_retry_queue.clone() {

            self.logger.log(
                format!("Handling fragment: [session: {} - fragment: {}]",
                        session_id,
                        fragment_id
                ).as_str(),
                DEBUG
            );

            // Fetch information regarding the fragment from the server
            // If there are fragments for the current session
            if let Some(fragments) = self.fragment_sent.get_mut(&session_id) {

                if let Some(packet) = fragments.iter().find(|&packet| {
                    match &packet.pack_type {
                        PacketType::MsgFragment(fragment) => {
                            fragment.fragment_index == fragment_id
                        }
                        _ => false
                    }
                }) {

                    // Compute new route, this way if a flood response has updated the topology
                    // the packet should avoid the previous error
                    let destination_id = packet.routing_header.destination();
                    if destination_id.is_none() {
                        self.logger.log(
                            format!(
                                "No destination found for the fragment [{}] in session [{}]",
                                fragment_id,
                                session_id
                            ).as_str(),
                            ERROR
                        );
                        continue;
                    }

                    let source_header = self.topology.get_routing_header(
                        self.id,
                        destination_id.unwrap()
                    );
                    if source_header.is_empty() {
                        self.logger.log(
                            format!(
                                "No route was computed to destination node [{}]",
                                destination_id.unwrap()
                            ).as_str(),
                            ERROR
                        );

                        self.start_flooding();
                        continue;
                    }

                    // Fetch the fragment from the packet
                    let PacketType::MsgFragment(fragment) = packet.clone().pack_type
                        else {
                            // Should never reach this code
                            self.logger.log(
                                "Expecting MsgFragment, received wrong type in resend_fragments",
                                ERROR
                            );
                            continue;
                        };

                    // Create a new packet and send it
                    let new_packet = Packet::new_fragment(
                        source_header,
                        session_id,
                        fragment
                    );

                    self.send_packet(new_packet);

                    // Remove the current tuple from the set
                    self.fragment_retry_queue.remove(&(session_id, fragment_id));
                } else {

                    // NO FRAGMENT FOUND
                    self.logger.log(
                        format!("No fragment with id [{}] was found in session [{}]",
                            fragment_id,
                            session_id
                        ).as_str(),
                        ERROR
                    );
                }
            } else {

                // NO SESSION FOUND
                self.logger.log(
                    format!("No session with id [{}] is registered", session_id).as_str(),
                    ERROR
                );
            }
        }
    }

    /// Run the server, listening for incoming commands from the simulation controller or incoming
    /// packets from the adjacent drones
    pub fn run(&mut self) {

        // Add himself to the topology
        self.topology.add_node(self.id);

        // Flood the first time
        self.start_flooding();

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


    // <== GETTERS AND SETTERS ==>
    pub fn topology(&self) -> &Topology { &self.topology }

    /// Utility method that updates the current topology of the server,
    /// adding the `nodes` and `edges`
    ///
    /// # Args
    /// * `nodes: Vec<NodeId>` - vector of nodes to add to the topology
    /// * `edges: Vec<(NodeId, NodeId)>` - vector of edges to add to the topology,
    /// both from the first node to the second and vice versa
    pub fn update_topology(&mut self, nodes: Vec<NodeId>, edges: Vec<(NodeId, NodeId)>) {

        // Ad node to node list if not there
        for node in nodes {
            if !self.topology.nodes().contains(&node) { self.topology.add_node(node); }
        }

        // Add edges between the two nodes, if not exists
        for edge in edges {
            if !self.topology.edges().contains_key(&edge.0) ||
            !self.topology.edges().get(&edge.0).unwrap().contains(&edge.1) {
                self.topology.add_edge(edge.0, edge.1);
                self.topology.add_edge(edge.1, edge.0);
            }
        }
    }

    pub fn registered_clients(&self) -> &HashSet<NodeId> { &self.registered_clients }

    pub fn neighbours(&self) ->&HashMap<NodeId, Sender<Packet>> { &self.node_senders }

    pub fn fragment_sent(&self) -> &HashMap<u64, Vec<Packet>> { &self.fragment_sent }

    /// Return true if the current timestamp is grater than
    /// the last flood starting timestamp + a shared timeout
    pub fn can_flood(&self) -> bool {
        let now = Utc::now();
        now.timestamp_millis() > self.last_flood_timestamp + TIMEOUT_BETWEEN_FLOODS_MS as i64
    }

    pub fn fragment_retry_queue(&self) -> &HashSet<(u64, u64)> { &self.fragment_retry_queue }

    /// Return a vector of all nodes in the topology and their correspondent PDR
    /// (for servers and clients should be 0)
    pub fn get_pdr_for_topology(&mut self) -> Vec<(NodeId, u64)> {
        let mut pdrs = Vec::new();

        for node in self.topology.nodes().clone() {
            pdrs.push((node, self.topology.pdr_for_node(node)));
        }
        pdrs
    }
    pub fn get_pdr_for_node(&mut self, node_id: NodeId) -> u64 {self.topology.pdr_for_node(node_id)}
}

// invia flood-request event