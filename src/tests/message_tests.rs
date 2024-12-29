#[cfg(test)]
#[allow(unused_imports, unreachable_code, unused_variables)]
pub mod message_test {
    use std::collections::HashMap;
    use crossbeam_channel::{unbounded, Receiver, Sender};
    use rand::{Rng};
    use rustafarian_shared::assembler::assembler::Assembler;
    use rustafarian_shared::assembler::disassembler::Disassembler;
    use rustafarian_shared::messages::chat_messages::{ChatRequest, ChatRequestWrapper, ChatResponse, ChatResponseWrapper};
    use rustafarian_shared::messages::commander_messages::{SimControllerCommand, SimControllerEvent, SimControllerResponseWrapper};
    use rustafarian_shared::messages::general_messages::{DroneSend, ServerTypeRequest};
    use wg_2024::network::{NodeId, SourceRoutingHeader};
    use wg_2024::packet::{Fragment, Packet, PacketType};
    use crate::chat_server::ChatServer;

    /// Init a test ChatServer with 2 drones connected to it
    ///
    /// # Return
    /// Returns the init server and receiver for the drones and sim controller
    fn init_test_network() -> (
        ChatServer,
        Receiver<Packet>,
        Receiver<Packet>,
        Receiver<SimControllerResponseWrapper>
    ) {

        // NEIGHBOURS CHANNELS
        let node_2: (Sender<Packet>, Receiver<Packet>) = unbounded();
        let node_3: (Sender<Packet>, Receiver<Packet>) = unbounded();

        let mut neighbours_map = HashMap::new();
        neighbours_map.insert(2 as NodeId, node_2.0);
        neighbours_map.insert(3 as NodeId, node_3.0);

        // SIM CONTROLLER CHANNELS
        let sim_controller_resp: (Sender<SimControllerResponseWrapper>, Receiver<SimControllerResponseWrapper>) = unbounded();
        let sim_controller_recv: Receiver<SimControllerCommand> = unbounded().1;

        // SERVER CHANNELS
        let server_channel: (Sender<Packet>, Receiver<Packet>) = unbounded();

        let mut server: ChatServer = ChatServer::new(
            1,
            sim_controller_recv,
            sim_controller_resp.0,
            server_channel.1,
            neighbours_map,
            true
        );

        server.update_topology(vec![1, 2, 3], vec![(1, 2), (1, 3)]);

        (server, node_2.1, node_3.1, sim_controller_resp.1)
    }

    #[test]
    fn should_handle_message_fragment() {

        let mut rng = rand::thread_rng();
        let (mut server, recv2, recv3, sc_recv) = init_test_network();
        let session_id: u64 = rng.gen();

        // Add fake nodes to the topology
        server.update_topology(vec![7, 8], vec![(3, 7), (7, 8)]);

        // Create a mock fragment with a test content
        let fragment_index = 1;
        let fragment = Fragment::from_string(
            fragment_index,
            3,
            String::from("test")
        );

        // Create a mock routing header for the packet, coming from the node 8
        let routing_header = SourceRoutingHeader::new(
            vec![8, 7, 3, 1],
            3
        );

        // Used later to check if the routing for the ACK is correct
        let mut route = routing_header.hops.clone();
        route.reverse();


        // Create a mock packet
        let packet = Packet::new_fragment(routing_header, session_id, fragment);
        let packet_type = packet.pack_type.clone();

        server.handle_received_packet(Ok(packet));

        // Check that the ACK has the correct information
        let received_packet = recv3.recv().unwrap();

        assert_eq!(session_id, received_packet.session_id);
        assert_eq!(route, received_packet.routing_header.hops);

        match received_packet.pack_type {
            PacketType::Ack(ack) => {
                assert_eq!(fragment_index, ack.fragment_index);
            }
            _ => { !panic!("Unexpected packet type"); }
        }

        // Check that the event sent to the sim controller is correct
        let SimControllerResponseWrapper::Event(sim_event) = sc_recv.recv().unwrap()
        else { !panic!("Unexpected event type"); };

        match sim_event {

            SimControllerEvent::PacketSent {
                    session_id: event_session_id,
                    packet_type: _
            } => {
                assert_eq!(session_id, event_session_id);
            }
            _ => { !panic!("Unexpected controller event type"); }
        }
    }

    // SERVER FUNCTIONALITIES
    #[test]
    fn should_register_client() {

        let mut rng = rand::thread_rng();
        let (
            mut server,
            recv2,
            recv3,
            sc_recv
        ) = init_test_network();
        let session_id: u64 = rng.gen();

        // Add fake nodes to the topology
        server.update_topology(vec![7, 8], vec![(3, 7), (7, 8)]);

        // Create a mock request and fragment it
        let mut disassembler = Disassembler::new();
        let request = ChatRequestWrapper::Chat(ChatRequest::Register(8));
        let fragments = disassembler.disassemble_message(
            request.stringify().into_bytes(),
            session_id
        );

        // Create a mock routing header for the packet, coming from the node 8
        let routing_header = SourceRoutingHeader::new(
            vec![8, 7, 3, 1],
            3
        );

        // Send fragments to the server
        for fragment in fragments {
            let packet = Packet::new_fragment(routing_header.clone(), session_id, fragment);
            server.handle_received_packet(Ok(packet));
        }

        // Check that the client (8) is registered
        assert!(server.registered_clients().contains(&8));

        // Check the first packet is an ACK
        let packet = recv3.recv().unwrap();
        match packet.pack_type {
            PacketType::Ack(ack) => {}
            _ => { !panic!("Unexpected packet type, was expecting and ACK"); }
        }

        // Reassemble the response and check it is a ClientRegistered response
        let mut assembler = Assembler::new();
        let packet = recv3.recv().unwrap();
        match packet.pack_type {
            PacketType::MsgFragment(fragment) => {
                if let Some(message) = assembler.add_fragment(fragment, session_id) {

                    let message = String::from_utf8_lossy(&message).to_string();
                    match ChatResponseWrapper::from_string(message) {
                        Ok(resp) => {
                            match resp {
                                ChatResponseWrapper::Chat(response) => {
                                    match response {
                                        ChatResponse::ClientRegistered => {}
                                        _ => { !panic!("Unexpected request type, was expecting ClientRegistered"); }
                                    }
                                }
                                _ => { !panic!("Unexpected response type, expected ChatResponseWrapper::Chat"); }
                            }
                        }
                        Err(_) => {
                            !panic!("Something went wrong while parsing the request");
                        }
                    }
                }
            }
            _ => { !panic!("Unexpected packet type, was expecting MsgFragment"); }
        }
    }

    #[test]
    fn should_get_server_type() {

        let mut rng = rand::thread_rng();
        let (
            mut server,
            recv2,
            recv3,
            sc_recv
        ) = init_test_network();
        let session_id: u64 = rng.gen();

        // Add fake nodes to the topology
        server.update_topology(vec![7, 8], vec![(3, 7), (7, 8)]);

        // Create a mock request and fragment it
        let mut disassembler = Disassembler::new();
        let request = ChatRequestWrapper::ServerType(ServerTypeRequest::ServerType);
        let fragments = disassembler.disassemble_message(
            request.stringify().into_bytes(),
            session_id
        );

        // Create a mock routing header for the packet, coming from the node 8
        let routing_header = SourceRoutingHeader::new(
            vec![8, 7, 3, 1],
            3
        );

        // Send fragments to the server
        for fragment in fragments {
            let packet = Packet::new_fragment(routing_header.clone(), session_id, fragment);
            server.handle_received_packet(Ok(packet));
        }

        // Check the first packet is an ACK
        let packet = recv3.recv().unwrap();
        match packet.pack_type {
            PacketType::Ack(ack) => {}
            _ => { !panic!("Unexpected packet type, was expecting ACK"); }
        }

        // Reassemble the response and check it is a ServerType response
        let mut assembler = Assembler::new();
        let packet = recv3.recv().unwrap();
        match packet.pack_type {
            PacketType::MsgFragment(fragment) => {
                if let Some(message) = assembler.add_fragment(fragment, session_id) {

                    let message = String::from_utf8_lossy(&message).to_string();
                    match ChatResponseWrapper::from_string(message) {
                        Ok(req) => {
                            match req {
                                ChatResponseWrapper::ServerType(_) => {
                                }
                                _ => { !panic!("Unexpected response type, was expecting ServerType"); }
                            }
                        }
                        Err(_) => {
                            !panic!("Something went wrong while parsing the response");
                        }
                    }
                }
            }
            _ => { !panic!("Unexpected packet type, was expecting MsgFragment"); }
        }
    }

    #[test]
    fn should_get_registered_clients() {

        let mut rng = rand::thread_rng();
        let (
            mut server,
            recv2,
            recv3,
            sc_recv
        ) = init_test_network();
        let session_id: u64 = rng.gen();

        // Add fake nodes to the topology
        server.update_topology(vec![7, 8], vec![(3, 7), (7, 8)]);

        // Create a mock request and fragment it
        let mut disassembler = Disassembler::new();

        // Register the client
        let request = ChatRequestWrapper::Chat(ChatRequest::Register(8));
        let fragments = disassembler.disassemble_message(
            request.stringify().into_bytes(),
            session_id
        );

        // Create a mock routing header for the packet, coming from the node 8
        let routing_header = SourceRoutingHeader::new(
            vec![8, 7, 3, 1],
            3
        );

        // Send fragments to the server
        for fragment in fragments {
            let packet = Packet::new_fragment(routing_header.clone(), session_id, fragment);
            server.handle_received_packet(Ok(packet));
        }

        // Obtain registered clients
        let request = ChatRequestWrapper::Chat(ChatRequest::ClientList);
        let fragments = disassembler.disassemble_message(
            request.stringify().into_bytes(),
            session_id
        );

        // Send fragments to the server
        for fragment in fragments {
            let packet = Packet::new_fragment(routing_header.clone(), session_id, fragment);
            server.handle_received_packet(Ok(packet));
        }

        // Discard the first two packets, should be ACK and Response for client registration
        let packet = recv3.recv().unwrap();
        let packet = recv3.recv().unwrap();

        // Check the first packet is an ACK
        let packet = recv3.recv().unwrap();
        match packet.pack_type {
            PacketType::Ack(ack) => {}
            _ => { !panic!("Unexpected packet type, was expecting and ACK"); }
        }

        // Reassemble the response and check it is a ClientRegistered response
        let mut assembler = Assembler::new();
        let packet = recv3.recv().unwrap();
        match packet.pack_type {
            PacketType::MsgFragment(fragment) => {
                if let Some(message) = assembler.add_fragment(fragment, session_id) {

                    let message = String::from_utf8_lossy(&message).to_string();
                    match ChatResponseWrapper::from_string(message) {
                        Ok(resp) => {
                            match resp {
                                ChatResponseWrapper::Chat(response) => {
                                    match response {
                                        ChatResponse::ClientList(list) => {
                                            assert!(list.contains(&8));
                                        }
                                        _ => { !panic!("Unexpected request type, was expecting ClientList"); }
                                    }
                                }
                                _ => { !panic!("Unexpected response type, expected ChatResponseWrapper::Chat"); }
                            }
                        }
                        Err(_) => {
                            !panic!("Something went wrong while parsing the request");
                        }
                    }
                }
            }
            _ => { !panic!("Unexpected packet type, was expecting MsgFragment"); }
        }
    }

    #[test]
    fn should_send_message_to_client() {

        let mut rng = rand::thread_rng();
        let (
            mut server,
            recv2,
            recv3,
            sc_recv
        ) = init_test_network();
        let session_id: u64 = rng.gen();

        // Add fake nodes to the topology, 8 and 9 are clients
        server.update_topology(vec![6, 7, 8, 9], vec![(3, 6), (3, 7), (6, 8), (7, 9)]);

        // Create a mock request and fragment it
        let mut disassembler = Disassembler::new();
        let message = "Test message".to_string();
        let request = ChatRequestWrapper::Chat(ChatRequest::SendMessage {
            from: 8,
            to: 9,
            message: message.clone(),
        });
        let fragments = disassembler.disassemble_message(
            request.stringify().into_bytes(),
            session_id
        );

        // Create a mock routing header for the packet, coming from the node 8
        let routing_header = SourceRoutingHeader::new(
            vec![8, 6, 3, 1],
            3
        );

        // Send fragments to the server
        for fragment in fragments {
            let packet = Packet::new_fragment(routing_header.clone(), session_id, fragment);
            server.handle_received_packet(Ok(packet));
        }

        // Check the first packet is an ACK
        let packet = recv3.recv().unwrap();
        match packet.pack_type {
            PacketType::Ack(ack) => {}
            _ => { !panic!("Unexpected packet type, was expecting and ACK"); }
        }

        let mut assembler = Assembler::new();

        // MESSAGE TO THE RECEIVER
        // Reassemble the response and check it is a MessageFrom response
        let packet = recv3.recv().unwrap();
        match packet.pack_type {
            PacketType::MsgFragment(fragment) => {
                if let Some(message) = assembler.add_fragment(fragment, session_id) {

                    let message = String::from_utf8_lossy(&message).to_string();
                    match ChatResponseWrapper::from_string(message) {
                        Ok(resp) => {
                            match resp {
                                ChatResponseWrapper::Chat(response) => {
                                    match response {
                                        ChatResponse::MessageFrom {
                                            from,
                                            message
                                        } => {
                                            // Check the sender and the message are correct
                                            assert_eq!(8, from);
                                            assert_eq!(message.clone().to_vec(), message);
                                        }
                                        _ => { !panic!("Unexpected request type, was expecting MessageFrom"); }
                                    }
                                }
                                _ => { !panic!("Unexpected response type, expected ChatResponseWrapper::Chat"); }
                            }
                        }
                        Err(_) => {
                            !panic!("Something went wrong while parsing the request");
                        }
                    }
                }
            }
            _ => { !panic!("Unexpected packet type, was expecting MsgFragment"); }
        }

        // CONFIRM MESSAGE TO THE SENDER
        // Reassemble the response and check it is a MessageSent response
        let packet = recv3.recv().unwrap();
        match packet.pack_type {
            PacketType::MsgFragment(fragment) => {
                if let Some(message) = assembler.add_fragment(fragment, session_id) {

                    let message = String::from_utf8_lossy(&message).to_string();
                    match ChatResponseWrapper::from_string(message) {
                        Ok(resp) => {
                            match resp {
                                ChatResponseWrapper::Chat(response) => {
                                    match response {
                                        ChatResponse::MessageSent => {}
                                        _ => { !panic!("Unexpected request type, was expecting MessageFrom"); }
                                    }
                                }
                                _ => { !panic!("Unexpected response type, expected ChatResponseWrapper::Chat"); }
                            }
                        }
                        Err(_) => {
                            !panic!("Something went wrong while parsing the request");
                        }
                    }
                }
            }
            _ => { !panic!("Unexpected packet type, was expecting MsgFragment"); }
        }
    }
}