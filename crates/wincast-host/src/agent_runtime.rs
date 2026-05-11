use std::net::{SocketAddr, TcpListener};

use wincast_protocol::config::HostConfig;

use crate::{agent, program::StdProgramRunner};

pub(crate) trait HostAgentRuntime {
    fn run(&mut self, config: &HostConfig) -> Result<SocketAddr, String>;
}

#[derive(Debug, Default)]
pub(crate) struct StdHostAgentRuntime;

impl HostAgentRuntime for StdHostAgentRuntime {
    fn run(&mut self, config: &HostConfig) -> Result<SocketAddr, String> {
        let listener = TcpListener::bind(&config.listen)
            .map_err(|error| format!("宿主端 TCP 监听失败: {error}"))?;
        let mut runner = StdProgramRunner;
        agent::run_control_listener(listener, config, &mut runner)
    }
}
