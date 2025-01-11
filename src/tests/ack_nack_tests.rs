#[cfg(test)]
#[allow(unused_imports, unreachable_code, unused_variables)]
pub mod ack_nack_tests {
    use std::collections::HashMap;
    use std::thread::sleep;
    use std::time::Duration;
    use crossbeam_channel::{unbounded, Receiver, Sender};
    use rand::{Rng};
    use rustafarian_shared::assembler::assembler::Assembler;
    use rustafarian_shared::assembler::disassembler::Disassembler;
    use rustafarian_shared::messages::chat_messages::{ChatRequest, ChatRequestWrapper, ChatResponse, ChatResponseWrapper};
    use rustafarian_shared::messages::commander_messages::{SimControllerCommand, SimControllerEvent, SimControllerResponseWrapper};
    use rustafarian_shared::messages::general_messages::{DroneSend, ServerTypeRequest};
    use rustafarian_shared::TIMEOUT_BETWEEN_FLOODS_MS;
    use wg_2024::network::{NodeId, SourceRoutingHeader};
    use wg_2024::packet::{FloodRequest, FloodResponse, Fragment, Nack, NackType, Packet, PacketType};
    use wg_2024::packet::NodeType::{Client, Drone, Server};
    use crate::chat_server::ChatServer;

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
    fn should_handle_ack() {
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

        // Create a mock request, using ServerType so the server sends a response back to the client
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

        let total_fragments = fragments.len();

        // Check that no fragment are present
        assert!(server.fragment_sent().is_empty());

        // Send fragments to the server
        for fragment in fragments.clone() {
            let packet = Packet::new_fragment(routing_header.clone(), session_id, fragment);
            server.handle_received_packet(Ok(packet));
        }

        // Check the fragment of the response are added to the list of fragment sent
        assert!(!server.fragment_sent().is_empty());

        // Send ACK to the server for each fragment
        for fragment in fragments.clone() {
            let ack = Packet::new_ack(
                routing_header.clone(),
                session_id,
                fragment.fragment_index
            );
            server.handle_received_packet(Ok(ack));
        }

        // Check the fragment are removed
        // Should be empty since a session is removed if no more fragments are present
        assert!(server.fragment_sent().is_empty());
    }

    #[test]
    fn should_handle_ack_multiple_packets() {
        let mut rng = rand::thread_rng();
        let (
            mut server,
            recv2,
            recv3,
            sc_recv
        ) = init_test_network();

        // Add fake nodes to the topology
        server.update_topology(vec![7, 8], vec![(3, 7), (7, 8)]);
        let mut disassembler = Disassembler::new();

        // Create a mock routing header for the packet, coming from the node 8
        let routing_header = SourceRoutingHeader::new(
            vec![8, 7, 3, 1],
            3
        );

        // SEND FIRST MESSAGE
        let session_id_1: u64 = rng.gen();
        let request = ChatRequestWrapper::ServerType(ServerTypeRequest::ServerType);
        let fragments_1 = disassembler.disassemble_message(
            request.stringify().into_bytes(),
            session_id_1
        );
        let total_fragments = fragments_1.len();

        // Send fragments to the server
        for fragment in fragments_1.clone() {
            let packet = Packet::new_fragment(routing_header.clone(), session_id_1, fragment);
            server.handle_received_packet(Ok(packet));
        }

        // SEND SECOND MESSAGE
        let session_id_2: u64 = rng.gen();
        let request = ChatRequestWrapper::ServerType(ServerTypeRequest::ServerType);
        let fragments_2 = disassembler.disassemble_message(
            request.stringify().into_bytes(),
            session_id_2
        );
        let total_fragments = fragments_2.len();

        // Send fragments to the server
        for fragment in fragments_2.clone() {
            let packet = Packet::new_fragment(routing_header.clone(), session_id_2, fragment);
            server.handle_received_packet(Ok(packet));
        }


        // Check the fragment of the response are added to the list of fragment sent
        assert!(!server.fragment_sent().is_empty());
        // Check there are 2 different entries
        assert_eq!(2, server.fragment_sent().keys().len());

        // Send ACK to the server for each fragment of the first message
        for fragment in fragments_1.clone() {
            let ack = Packet::new_ack(
                routing_header.clone(),
                session_id_1,
                fragment.fragment_index
            );
            server.handle_received_packet(Ok(ack));
        }

        // Check that only the fragment of the first message are removed
        assert_eq!(1, server.fragment_sent().keys().len());
        assert!(server.fragment_sent().contains_key(&session_id_2));

        // Send ACK to the server for each fragment of the second message
        for fragment in fragments_2.clone() {
            let ack = Packet::new_ack(
                routing_header.clone(),
                session_id_2,
                fragment.fragment_index
            );
            server.handle_received_packet(Ok(ack));
        }
        assert!(server.fragment_sent().is_empty());
    }

    #[test]
    fn should_handle_ack_wrong_session_id() {
        let (
            mut server,
            recv2,
            recv3,
            sc_recv
        ) = init_test_network();
        let session_id: u64 = 12345;

        // Add fake nodes to the topology
        server.update_topology(vec![7, 8], vec![(3, 7), (7, 8)]);

        // Create a mock request, using ServerType so the server sends a response back to the client
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
        let total_fragments = fragments.len();

        // Send fragments to the server
        for fragment in fragments.clone() {
            let packet = Packet::new_fragment(routing_header.clone(), session_id, fragment);
            server.handle_received_packet(Ok(packet));
        }

        let wrong_ses_id = 67890;
        // Send ACK to the server for each fragment
        for fragment in fragments.clone() {
            let ack = Packet::new_ack(
                routing_header.clone(),
                wrong_ses_id,
                fragment.fragment_index
            );
            server.handle_received_packet(Ok(ack));
        }

        // Check the fragment are not removed
        assert_eq!(1, server.fragment_sent().keys().len());
        assert!(server.fragment_sent().contains_key(&session_id));
    }

    #[test]
    fn should_handle_ack_wrong_fragment_id() {
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

        // Create a mock request, using ServerType so the server sends a response back to the client
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
        let total_fragments = fragments.len();

        // Send fragments to the server
        for fragment in fragments.clone() {
            let packet = Packet::new_fragment(routing_header.clone(), session_id, fragment);
            server.handle_received_packet(Ok(packet));
        }

        // Send ACK to the server for the wrong fragment_id
        let fragment_index: u64 = (total_fragments + 50) as u64;
        let ack = Packet::new_ack(routing_header.clone(), session_id, fragment_index);
        server.handle_received_packet(Ok(ack));

        // Check that no fragment is removed
        assert!(!server.fragment_sent().is_empty());
        assert!(!server.fragment_sent().get(&session_id).unwrap().is_empty())
    }

    #[test]
    fn should_handle_nack() {
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

        // <== SEND A MESSAGE TO THE SERVER ==>

        // Create a mock request, using ServerType so the server sends a response back to the client
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

        let total_fragments = fragments.len();

        // Check that no fragment are present
        assert!(server.fragment_sent().is_empty());

        // Send fragments to the server
        for fragment in fragments.clone() {
            let packet = Packet::new_fragment(routing_header.clone(), session_id, fragment);
            server.handle_received_packet(Ok(packet));
        }

        // Check the fragment of the response are added to the list of fragment sent
        assert!(!server.fragment_sent().is_empty());
        // Check the server is not currently flooding
        assert!(server.can_flood());

        // <== INTERCEPT RESPONSE AND SEND NACK ==>

        // Remove any packet in the channels, to avoid problems
        // on the assertions for the flood request
        while let Ok(_) = recv2.try_recv() {}

        // Add response fragments to a list and send a NACK
        let mut resp_fragments: Vec<Fragment> = Vec::new();
        while let Ok(response) = recv3.try_recv() {

            match response.pack_type {
                PacketType::MsgFragment(fragment) => {
                    resp_fragments.push(fragment);
                }
                _ => {}
            }
        }

        for fragment in resp_fragments.clone() {
            let nack = Packet::new_nack(
                routing_header.clone(),
                session_id,
                Nack{
                    fragment_index: fragment.fragment_index,
                    nack_type: NackType::ErrorInRouting(7)
                }
            );
            server.handle_received_packet(Ok(nack));
        }

        // Check the fragment are not removed
        assert!(!server.fragment_sent().is_empty());

        // Check the fragment are added to the retry_later queue
        assert!(!server.fragment_retry_queue().is_empty());
        for fragment in fragments {
            assert!(
                server
                .fragment_retry_queue()
                .contains(&(session_id, fragment.fragment_index))
            );
        }

        // <== SERVER START FLOOD AND AWAIT A RESPONSE ==>

        assert!(!server.topology().nodes().contains(&7));

        // Check the server has started a new flood
        assert!(!server.can_flood());

        let fr1 = recv2.recv().unwrap();
        let fr2 = recv3.recv().unwrap();

        match fr1.pack_type {
            PacketType::FloodRequest(fr) => {
                assert_eq!(1, fr.initiator_id);
            }
            _ => {
                panic!("Wrong packet type for drone 2");
            }
        }

        match fr2.pack_type {
            PacketType::FloodRequest(fr) => {
                assert_eq!(1, fr.initiator_id);
            }
            _ => {
                panic!("Wrong packet type for drone 3");
            }
        }

        // Create a mock flood response, simulating a response from the flood
        let flood_id: u64 = rng.gen();
        let flood_request = FloodRequest {
            flood_id,
            initiator_id: 8,
            path_trace: vec![(1, Server), (3, Drone), (7, Drone), (8, Client)],
        };

        // Compute the route, used to send the response back through the network
        let mut route: Vec<u8> = flood_request.path_trace
            .iter()
            .map(|node| node.0)
            .collect();
        route.reverse();

        let flood_response = Packet::new_flood_response(
            SourceRoutingHeader::new(route, 3),
            session_id,
            FloodResponse {
                flood_id,
                path_trace: flood_request.path_trace
            }
        );
        server.handle_received_packet(Ok(flood_response));

        // <== SERVER RE-SEND FRAGMENTS ==>

        let resend_packet = recv3.recv().unwrap();
        assert_eq!(session_id, resend_packet.session_id);
        match resend_packet.pack_type {
            PacketType::MsgFragment(fragment) => {
                assert_eq!(*resp_fragments.get(fragment.fragment_index as usize).unwrap(), fragment);
            }
            _ => {
                panic!("Was expecting MsgFragment as packet type")
            }
        }
    }

    #[test]
    fn should_handle_nack_on_packet_dropped() {
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

        // Create a mock request, using ServerType so the server sends a response back to the client
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

        let total_fragments = fragments.len();

        // Check that no fragment are present
        assert!(server.fragment_sent().is_empty());

        // Send fragments to the server
        for fragment in fragments.clone() {
            let packet = Packet::new_fragment(routing_header.clone(), session_id, fragment);
            server.handle_received_packet(Ok(packet));
        }

        // Check the fragment of the response are added to the list of fragment sent
        assert!(!server.fragment_sent().is_empty());
        // Check the server is not currently flooding
        assert!(server.can_flood());

        // Check that the pdr for the node 3 is 0
        assert_eq!(0, server.get_pdr_for_node(3));

        // Remove any packet in the channels, to avoid problems
        // on the assertions for the flood request
        while let Ok(_) = recv2.try_recv() {}
        while let Ok(_) = recv3.try_recv() {}

        // Send NACK to the server for each fragment
        for fragment in fragments.clone() {
            let nack = Packet::new_nack(
                SourceRoutingHeader::new(
                    vec![3, 1],
                    1
                ),
                session_id,
                Nack{
                    fragment_index: fragment.fragment_index,
                    nack_type: NackType::Dropped
                }
            );
            server.handle_received_packet(Ok(nack));
        }

        // Check the fragment are not removed and are not added to the retry_queue
        assert!(!server.fragment_sent().is_empty());
        assert!(server.fragment_retry_queue().is_empty());

        // Check the server has not started a new flood
        assert!(server.can_flood());

        // Check that the history of the node 3 is updates, pdr should now be 50% since
        // it has now sent 2 packets and dropped 1
        assert_eq!(50, server.get_pdr_for_node(3));

        while let Ok(response) = recv3.try_recv() {

            // Check the packet is the right one
            assert_eq!(session_id, response.session_id);
            assert_eq!(1, response.routing_header.hops[0]);
            match response.pack_type {
                PacketType::MsgFragment(fr) => {}
                _ => {
                    panic!("Wrong packet type for drone 2");
                }
            }
        }
    }
}