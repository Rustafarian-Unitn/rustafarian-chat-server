#[cfg(test)]
#[allow(unused_imports, unreachable_code, unused_variables)]
pub mod sc_command_tests {
    use crate::chat_server::ChatServer;
    use crossbeam_channel::{unbounded, Receiver, Sender};
    use rand::Rng;
    use rustafarian_shared::messages::commander_messages::{
        SimControllerCommand, SimControllerMessage, SimControllerResponseWrapper,
    };
    use std::collections::HashMap;
    use wg_2024::network::{NodeId, SourceRoutingHeader};
    use wg_2024::packet::NodeType::{Client, Drone, Server};
    use wg_2024::packet::{FloodRequest, FloodResponse, NodeType, Packet, PacketType};

    fn init_test_network() -> (ChatServer, Receiver<SimControllerResponseWrapper>) {
        // NEIGHBOURS CHANNELS
        let node_2: (Sender<Packet>, Receiver<Packet>) = unbounded();

        let mut neighbours_map = HashMap::new();
        neighbours_map.insert(2 as NodeId, node_2.0);

        // SIM CONTROLLER CHANNELS
        let sim_controller_resp: (
            Sender<SimControllerResponseWrapper>,
            Receiver<SimControllerResponseWrapper>,
        ) = unbounded();
        let sim_controller_recv: (Sender<SimControllerCommand>, Receiver<SimControllerCommand>) =
            unbounded();

        // SERVER CHANNELS
        let server_channel: (Sender<Packet>, Receiver<Packet>) = unbounded();

        let server: ChatServer = ChatServer::new(
            1,
            sim_controller_recv.1,
            sim_controller_resp.0,
            server_channel.1,
            neighbours_map,
            true,
        );

        (server, sim_controller_resp.1)
    }

    #[test]
    fn should_handle_add_sender_command() {
        let (mut server, _) = init_test_network();
        // Add himself to the topology, this is normally done in the .run method
        server.update_topology(vec![(1, NodeType::Server)], vec![]);

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
        let (mut server, _) = init_test_network();
        // Add himself to the topology, this is normally done in the .run method
        server.update_topology(vec![(1, NodeType::Server)], vec![]);

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

        // Topology should still contain the new node, but there should not be any edges
        // between the node and the server
        assert_eq!(2, topology.nodes().len());
        assert!(topology.nodes().contains(&node_id));

        // Check there is an edge between the node and the server and vice versa
        assert!(topology.edges().contains_key(&node_id));
        assert!(!topology.edges().get(&node_id).unwrap().contains(&1));
        assert!(!topology.edges().get(&1).unwrap().contains(&node_id));
    }

    #[test]
    fn should_handle_topology_command() {
        let (mut server, sc_recv) = init_test_network();
        // Add himself to the topology, this is normally done in the .run method
        server.update_topology(vec![(1, NodeType::Server)], vec![]);

        // Add a neighbour
        let node_id: NodeId = 3;
        let node3_channel: (Sender<Packet>, Receiver<Packet>) = unbounded();
        let command = SimControllerCommand::AddSender(node_id, node3_channel.0);
        server.handle_controller_commands(Ok(command));

        // Read the flood_request_sent event
        sc_recv.recv().unwrap();

        let command = SimControllerCommand::Topology;
        server.handle_controller_commands(Ok(command));

        let response = sc_recv.recv().unwrap();

        match response {
            SimControllerResponseWrapper::Message(msg) => {
                match msg {
                    SimControllerMessage::TopologyResponse(topology) => {
                        // Check the topology is correct
                        assert_eq!(2, topology.nodes().len());
                        assert!(topology.nodes().contains(&node_id));

                        // Check there is an edge between the node and the server and vice versa
                        assert!(topology.edges().get(&node_id).unwrap().contains(&1));
                        assert!(topology.edges().get(&1).unwrap().contains(&node_id));
                    }
                    _ => {
                        panic!("Unexpected SimControllerMessage type")
                    }
                }
            }
            SimControllerResponseWrapper::Event(_) => {
                panic!("Unexpected SimControllerResponseWrapper type")
            }
        }
    }
}
