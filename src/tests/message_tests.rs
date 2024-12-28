#[cfg(test)]
pub mod message_test {
    use std::collections::HashMap;
    use std::fmt::Debug;
    use crossbeam_channel::{unbounded, Receiver, Sender};
    use rand::{Rng};
    use rustafarian_shared::messages::commander_messages::{SimControllerCommand, SimControllerEvent, SimControllerResponseWrapper};
    use wg_2024::network::{NodeId, SourceRoutingHeader};
    use wg_2024::packet::{FloodRequest, FloodResponse, Fragment, Packet, PacketType};
    use wg_2024::packet::NodeType::{Client, Drone, Server};
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

    #[test]
    fn should_handle_complete_message_fragment() {

        let mut rng = rand::thread_rng();
        let (mut server, recv2, recv3, sc_recv) = init_test_network();
        let session_id: u64 = rng.gen();

        // Add fake nodes to the topology
        server.update_topology(vec![7, 8], vec![(3, 7), (7, 8)]);

        // Create a mock fragment with a test content
        let fragment_index = 1;
        let fragment = Fragment::from_string(
            fragment_index,
            1,
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

        // TODO Implement
    }
}