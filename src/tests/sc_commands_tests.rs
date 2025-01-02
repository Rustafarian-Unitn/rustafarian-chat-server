#[cfg(test)]
#[allow(unused_imports, unreachable_code, unused_variables)]
pub mod sc_command_tests {
    use std::collections::HashMap;
    use crossbeam_channel::{unbounded, Receiver, Sender};
    use rand::{Rng};
    use rustafarian_shared::messages::commander_messages::{SimControllerCommand, SimControllerResponseWrapper};
    use wg_2024::network::{NodeId, SourceRoutingHeader};
    use wg_2024::packet::{FloodRequest, FloodResponse, Packet, PacketType};
    use wg_2024::packet::NodeType::{Client, Drone, Server};
    use crate::chat_server::ChatServer;

    /// Init a test ChatServer with 2 drones connected to it
    ///
    /// # Return
    /// Returns the init server
    fn init_test_network() -> ChatServer {

        // NEIGHBOURS CHANNELS
        let node_2: (Sender<Packet>, Receiver<Packet>) = unbounded();

        let mut neighbours_map = HashMap::new();
        neighbours_map.insert(2 as NodeId, node_2.0);

        // SIM CONTROLLER CHANNELS
        let sim_controller_resp: (Sender<SimControllerResponseWrapper>, Receiver<SimControllerResponseWrapper>) = unbounded();
        let sim_controller_recv: (Sender<SimControllerCommand>, Receiver<SimControllerCommand>) = unbounded();

        // SERVER CHANNELS
        let server_channel: (Sender<Packet>, Receiver<Packet>) = unbounded();

        let server: ChatServer = ChatServer::new(
            1,
            sim_controller_recv.1,
            sim_controller_resp.0,
            server_channel.1,
            neighbours_map,
            true
        );

        server
    }

    #[test]
    fn should_handle_add_sender_command() {

        let mut server = init_test_network();
        // Add himself to the topology, this is normally done in the .run method
        server.update_topology(vec![1], vec![]);

        let node_id: NodeId = 3;
        let node3_channel: (Sender<Packet>, Receiver<Packet>) = unbounded();
        let command = SimControllerCommand::AddSender(node_id, node3_channel.0);

        server.handle_controller_commands(Ok(command));

        // Check the node has been added to the neighbours
        assert!(server.neighbours().contains_key(&node_id));

        // Check the topology has been updated
        let topology = server.topology();

        assert_eq!(2, topology.nodes().len());
        assert!(topology.nodes().contains(&node_id));

        // Check there is an edge between the node and the server and vice versa
        assert!(topology.edges().get(&node_id).unwrap().contains(&1));
        assert!(topology.edges().get(&1).unwrap().contains(&node_id));
    }

    #[test]
    fn should_handle_remove_sender_command() {

        let mut server = init_test_network();
        // Add himself to the topology, this is normally done in the .run method
        server.update_topology(vec![1], vec![]);

        // Add a neighbour
        let node_id: NodeId = 3;
        let node3_channel: (Sender<Packet>, Receiver<Packet>) = unbounded();
        let command = SimControllerCommand::AddSender(node_id, node3_channel.0);
        server.handle_controller_commands(Ok(command));

        // Remove the neighbour
        let command = SimControllerCommand::RemoveSender(node_id);
        server.handle_controller_commands(Ok(command));

        assert!(!server.neighbours().contains_key(&node_id));

        // Check the topology has been updated
        let topology = server.topology();

        assert_eq!(1, topology.nodes().len());
        assert!(!topology.nodes().contains(&node_id));

        // Check there is an edge between the node and the server and vice versa
        assert!(!topology.edges().contains_key(&node_id));
        assert!(!topology.edges().get(&1).unwrap().contains(&node_id));
    }
}