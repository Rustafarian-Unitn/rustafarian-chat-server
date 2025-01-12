#[cfg(test)]
#[allow(unused_imports, unreachable_code, unused_variables)]
pub mod flooding_tests {
    use crate::chat_server::ChatServer;
    use crossbeam_channel::{unbounded, Receiver, Sender};
    use rand::Rng;
    use rustafarian_shared::assembler::disassembler::Disassembler;
    use rustafarian_shared::messages::chat_messages::ChatRequestWrapper;
    use rustafarian_shared::messages::commander_messages::{
        SimControllerCommand, SimControllerResponseWrapper,
    };
    use rustafarian_shared::messages::general_messages::{DroneSend, ServerTypeRequest};
    use rustafarian_shared::TIMEOUT_BETWEEN_FLOODS_MS;
    use std::collections::HashMap;
    use std::thread::sleep;
    use std::time::Duration;
    use wg_2024::network::{NodeId, SourceRoutingHeader};
    use wg_2024::packet::NodeType::{Client, Drone, Server};
    use wg_2024::packet::{FloodRequest, FloodResponse, NodeType, Packet, PacketType};

    fn init_test_network() -> (
        ChatServer,
        Receiver<Packet>,
        Receiver<Packet>,
        Receiver<SimControllerResponseWrapper>,
    ) {
        // NEIGHBOURS CHANNELS
        let node_2: (Sender<Packet>, Receiver<Packet>) = unbounded();
        let node_3: (Sender<Packet>, Receiver<Packet>) = unbounded();

        let mut neighbours_map = HashMap::new();
        neighbours_map.insert(2 as NodeId, node_2.0);
        neighbours_map.insert(3 as NodeId, node_3.0);

        // SIM CONTROLLER CHANNELS
        // SIM CONTROLLER CHANNELS
        let sim_controller_resp: (
            Sender<SimControllerResponseWrapper>,
            Receiver<SimControllerResponseWrapper>,
        ) = unbounded();
        let sim_controller_recv: Receiver<SimControllerCommand> = unbounded().1;

        // SERVER CHANNELS
        let server_channel: (Sender<Packet>, Receiver<Packet>) = unbounded();

        let server: ChatServer = ChatServer::new(
            1,
            sim_controller_recv,
            sim_controller_resp.0,
            server_channel.1,
            neighbours_map,
            true,
        );

        (server, node_2.1, node_3.1, sim_controller_resp.1)
    }

    #[test]
    fn should_handle_flood_request() {
        let mut rng = rand::thread_rng();
        let (mut server, recv2, recv3, _) = init_test_network();
        let session_id: u64 = rng.gen();
        let flood_id: u64 = rng.gen();

        // Create a mock flood request, generated by a non-existing client and received through
        // Drone 3
        let flood_request = FloodRequest {
            flood_id,
            initiator_id: 8,
            path_trace: vec![(8, Client), (7, Drone), (3, Drone)],
        };

        // Create a new packet with the flood request
        let flood_request_packet = Packet::new_flood_request(
            SourceRoutingHeader::empty_route(),
            session_id,
            flood_request.clone(),
        );

        server.handle_received_packet(Ok(flood_request_packet));

        // Receive the flood request from the server
        let recv_2 = recv2.recv().unwrap();

        // Assert they are the same request, with same session id
        assert_eq!(session_id, recv_2.session_id);

        // Just check the packet is a flood request and check the content
        assert!(matches!(recv_2.pack_type, PacketType::FloodRequest(_)));
        match recv_2.pack_type {
            PacketType::FloodRequest(recv_flood_request) => {
                assert_eq!(flood_request.flood_id, recv_flood_request.flood_id);
                assert_eq!(flood_request.initiator_id, recv_flood_request.initiator_id);
                assert!(recv_flood_request.path_trace.contains(&(1, Server)))
            }
            _ => {
                !panic!("Unexpected packet type");
            }
        }

        // Check that no message is sent on the recv3 channel
        assert!(recv3.try_recv().is_err());
    }

    #[test]
    fn should_handle_flood_response() {
        let mut rng = rand::thread_rng();
        let (mut server, recv2, recv3, _) = init_test_network();
        let session_id: u64 = rng.gen();
        let flood_id: u64 = rng.gen();

        let flood_request = FloodRequest {
            flood_id,
            initiator_id: 8,
            path_trace: vec![(1, Server), (3, Drone), (7, Drone), (8, Client)],
        };

        // Compute the route, used to send the response back through the network
        let mut route: Vec<u8> = flood_request.path_trace.iter().map(|node| node.0).collect();
        route.reverse();

        let flood_response = Packet::new_flood_response(
            SourceRoutingHeader::new(route, 3),
            session_id,
            FloodResponse {
                flood_id,
                path_trace: flood_request.path_trace,
            },
        );

        server.handle_received_packet(Ok(flood_response));

        // Check that the server correctly updated the flood status
        assert!(server.can_flood());

        // Assert the topology is correctly updated
        // NODES
        assert!(server.topology().nodes().contains(&3));
        assert!(server.topology().nodes().contains(&7));
        assert!(server.topology().nodes().contains(&8));

        // EDGES
        assert!(server.topology().edges().get(&3).unwrap().contains(&7));
        assert!(server.topology().edges().get(&7).unwrap().contains(&3));
        assert!(server.topology().edges().get(&7).unwrap().contains(&8));
        assert!(server.topology().edges().get(&8).unwrap().contains(&7));
    }

    #[test]
    fn should_handle_flood_response_not_initiator() {
        let mut rng = rand::thread_rng();
        let (mut server, recv2, recv3, sc_receiver) = init_test_network();
        let session_id: u64 = rng.gen();
        let flood_id: u64 = rng.gen();

        // Server 9 is the initiator, the response is passing through the current server, received
        // from Node 3 and should be sent through Node 2
        let flood_request = FloodRequest {
            flood_id,
            initiator_id: 8,
            path_trace: vec![
                (9, Server),
                (2, Drone),
                (1, Server),
                (3, Drone),
                (7, Drone),
                (8, Client),
            ],
        };

        // Compute the route, used to send the response back through the network
        let mut route: Vec<u8> = flood_request.path_trace.iter().map(|node| node.0).collect();
        route.reverse();

        let flood_response = Packet::new_flood_response(
            SourceRoutingHeader::new(route, 3),
            session_id,
            FloodResponse {
                flood_id,
                path_trace: flood_request.path_trace,
            },
        );

        server.handle_received_packet(Ok(flood_response));

        // Check the response is forwarded to the right node
        let recv_2 = recv2.recv().unwrap();
        // Assert they are the same request, with same session id
        assert_eq!(session_id, recv_2.session_id);
        // Check that the hop_index has been updated
        assert_eq!(4, recv_2.routing_header.hop_index);
    }

    #[test]
    fn should_not_flood_flood_timeout() {
        let mut rng = rand::thread_rng();
        let (mut server, recv2, recv3, sc_recv) = init_test_network();
        let session_id: u64 = rng.gen();

        // Add fake nodes to the topology
        server.update_topology(vec![(7, Drone), (8, Drone)], vec![(3, 7)]);

        // Create a mock request, from a client which is not connected, to start a new flood
        let mut disassembler = Disassembler::new();
        let request = ChatRequestWrapper::ServerType(ServerTypeRequest::ServerType);
        let fragments =
            disassembler.disassemble_message(request.stringify().into_bytes(), session_id);
        // Create a mock routing header for the packet, coming from the node 8
        let routing_header = SourceRoutingHeader::new(vec![8, 7, 3, 1], 3);

        // Server should be able to flood now, since the start method is not called, the server
        // shouldn't have any flood active
        assert!(server.can_flood());

        // Send fragments to the server
        for fragment in fragments.clone() {
            let packet = Packet::new_fragment(routing_header.clone(), session_id, fragment);
            server.handle_received_packet(Ok(packet));
        }

        // Here the server shouldn't be able to flood, since a new request will start once
        // the server finds no route to the destination node (8)
        assert!(!server.can_flood());

        // Check that after the timeout + a small delta the server is once again able to start a flood
        sleep(Duration::from_millis(TIMEOUT_BETWEEN_FLOODS_MS + 50));
        assert!(server.can_flood());
    }
}
