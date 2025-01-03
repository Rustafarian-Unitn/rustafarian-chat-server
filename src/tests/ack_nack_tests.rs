#[cfg(test)]
#[allow(unused_imports, unreachable_code, unused_variables)]
pub mod ack_nack_tests {
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
}