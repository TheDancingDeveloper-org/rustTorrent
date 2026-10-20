use anyhow::Result;
use bollard::Docker;
use futures_util::StreamExt;
use std::collections::HashMap;

/// Connect to the Docker daemon via the Unix socket.
pub fn connect() -> Result<Docker> {
    Ok(Docker::connect_with_unix_defaults()?)
}

/// Get IPs of all containers with a given compose service label.
pub async fn get_service_ips(docker: &Docker, service: &str) -> Result<Vec<String>> {
    let filters = HashMap::from([(
        "label".to_string(),
        vec![format!("com.docker.compose.service={service}")],
    )]);
    let containers = docker
        .list_containers(Some(bollard::query_parameters::ListContainersOptions {
            filters: Some(filters),
            ..Default::default()
        }))
        .await?;

    let mut ips = Vec::new();
    for c in containers {
        if let Some(net_settings) = c.network_settings {
            if let Some(networks) = net_settings.networks {
                for (_name, net) in networks {
                    if let Some(ip) = net.ip_address {
                        if !ip.is_empty() {
                            ips.push(ip);
                            break;
                        }
                    }
                }
            }
        }
    }
    Ok(ips)
}

/// Find the container ID for a compose service.
pub async fn get_container_id(docker: &Docker, service: &str) -> Option<String> {
    let filters = HashMap::from([(
        "label".to_string(),
        vec![format!("com.docker.compose.service={service}")],
    )]);
    let containers = docker
        .list_containers(Some(bollard::query_parameters::ListContainersOptions {
            filters: Some(filters),
            ..Default::default()
        }))
        .await
        .ok()?;
    containers.first()?.id.clone()
}

/// Execute a command in a running container and return stdout.
pub async fn exec_in_container(
    docker: &Docker,
    container_id: &str,
    cmd: Vec<&str>,
) -> Result<String> {
    let exec = docker
        .create_exec(
            container_id,
            bollard::exec::CreateExecOptions {
                attach_stdout: Some(true),
                attach_stderr: Some(true),
                cmd: Some(cmd.iter().map(|s| s.to_string()).collect()),
                ..Default::default()
            },
        )
        .await?;

    let mut output = String::new();
    if let bollard::exec::StartExecResults::Attached { output: mut stream, .. } =
        docker.start_exec(&exec.id, None).await?
    {
        while let Some(Ok(chunk)) = stream.next().await {
            match chunk {
                bollard::container::LogOutput::StdOut { message }
                | bollard::container::LogOutput::StdErr { message } => {
                    output.push_str(&String::from_utf8_lossy(&message));
                }
                _ => {}
            }
        }
    }
    Ok(output)
}
