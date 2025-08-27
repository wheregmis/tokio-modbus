// SPDX-FileCopyrightText: Copyright (c) 2017-2025 slowtec GmbH <post@slowtec.de>
// SPDX-License-Identifier: MIT OR Apache-2.0

//! # TCP server example with connection limiting
//!
//! This example shows how to start a server with a maximum number of concurrent connections.
//! When the limit is reached, new connections will be rejected.

use std::{
    collections::HashMap,
    future,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::net::TcpListener;

use tokio_modbus::{
    prelude::*,
    server::tcp::{accept_tcp_connection, Server},
};

struct ExampleService {
    input_registers: Arc<Mutex<HashMap<u16, u16>>>,
    holding_registers: Arc<Mutex<HashMap<u16, u16>>>,
}

impl tokio_modbus::server::Service for ExampleService {
    type Request = Request<'static>;
    type Response = Response;
    type Exception = ExceptionCode;
    type Future = future::Ready<Result<Self::Response, Self::Exception>>;

    fn call(&self, req: Self::Request) -> Self::Future {
        let res = match req {
            Request::ReadInputRegisters(addr, cnt) => {
                register_read(&self.input_registers.lock().unwrap(), addr, cnt)
                    .map(Response::ReadInputRegisters)
            }
            Request::ReadHoldingRegisters(addr, cnt) => {
                register_read(&self.holding_registers.lock().unwrap(), addr, cnt)
                    .map(Response::ReadHoldingRegisters)
            }
            Request::WriteMultipleRegisters(addr, values) => {
                register_write(&mut self.holding_registers.lock().unwrap(), addr, &values)
                    .map(|_| Response::WriteMultipleRegisters(addr, values.len() as u16))
            }
            Request::WriteSingleRegister(addr, value) => register_write(
                &mut self.holding_registers.lock().unwrap(),
                addr,
                std::slice::from_ref(&value),
            )
            .map(|_| Response::WriteSingleRegister(addr, value)),
            _ => {
                println!("SERVER: Exception::IllegalFunction - Unimplemented function code in request: {req:?}");
                Err(ExceptionCode::IllegalFunction)
            }
        };
        future::ready(res)
    }
}

impl ExampleService {
    fn new() -> Self {
        // Insert some test data as register values.
        let mut input_registers = HashMap::new();
        input_registers.insert(0, 1234);
        input_registers.insert(1, 5678);
        let mut holding_registers = HashMap::new();
        holding_registers.insert(0, 10);
        holding_registers.insert(1, 20);
        holding_registers.insert(2, 30);
        holding_registers.insert(3, 40);
        Self {
            input_registers: Arc::new(Mutex::new(input_registers)),
            holding_registers: Arc::new(Mutex::new(holding_registers)),
        }
    }
}

/// Helper function implementing reading registers from a HashMap.
fn register_read(
    registers: &HashMap<u16, u16>,
    addr: u16,
    cnt: u16,
) -> Result<Vec<u16>, ExceptionCode> {
    let mut response_values = vec![0; cnt.into()];
    for i in 0..cnt {
        let reg_addr = addr + i;
        if let Some(r) = registers.get(&reg_addr) {
            response_values[i as usize] = *r;
        } else {
            println!("SERVER: Exception::IllegalDataAddress");
            return Err(ExceptionCode::IllegalDataAddress);
        }
    }

    Ok(response_values)
}

/// Write a holding register. Used by both the write single register
/// and write multiple registers requests.
fn register_write(
    registers: &mut HashMap<u16, u16>,
    addr: u16,
    values: &[u16],
) -> Result<(), ExceptionCode> {
    for (i, value) in values.iter().enumerate() {
        let reg_addr = addr + i as u16;
        if let Some(r) = registers.get_mut(&reg_addr) {
            *r = *value;
        } else {
            println!("SERVER: Exception::IllegalDataAddress");
            return Err(ExceptionCode::IllegalDataAddress);
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket_addr = "0.0.0.0:5503".parse().unwrap();
    const MAX_CONNECTIONS: usize = 2;

    tokio::select! {
        _ = server_context(socket_addr, MAX_CONNECTIONS) => unreachable!(),
        // _ = client_context(socket_addr) => println!("Exiting"),
    }

    Ok(())
}

async fn server_context(socket_addr: SocketAddr, max_connections: usize) -> anyhow::Result<()> {
    println!("Starting up server on {socket_addr} with max {max_connections} connections");
    let listener = TcpListener::bind(socket_addr).await?;
    let server = Server::with_connection_limit(listener, max_connections);
    let new_service = |_socket_addr| Ok(Some(ExampleService::new()));
    let on_connected = |stream, socket_addr| async move {
        accept_tcp_connection(stream, socket_addr, new_service)
    };
    let on_process_error = |err| {
        eprintln!("{err}");
    };
    server.serve(&on_connected, on_process_error).await?;
    Ok(())
}

async fn client_context(socket_addr: SocketAddr) {
    // Give the server some time for starting up
    tokio::time::sleep(Duration::from_secs(1)).await;

    // Create multiple clients to test connection limiting
    let mut handles = Vec::new();

    for client_id in 1..=5 {
        let handle = tokio::spawn(async move {
            println!("CLIENT {client_id}: Attempting to connect...");

            match tcp::connect(socket_addr).await {
                Ok(mut ctx) => {
                    println!("CLIENT {client_id}: Connected successfully!");

                    // Hold the connection for a few seconds
                    tokio::time::sleep(Duration::from_secs(2)).await;

                    // Try to read some registers
                    match ctx.read_input_registers(0x00, 2).await {
                        Ok(response) => {
                            println!("CLIENT {client_id}: Read successful: {response:?}");
                        }
                        Err(e) => {
                            println!("CLIENT {client_id}: Read failed: {e}");
                        }
                    }

                    tokio::time::sleep(Duration::from_secs(1)).await;
                    println!("CLIENT {client_id}: Disconnecting");
                }
                Err(e) => {
                    println!("CLIENT {client_id}: Connection failed: {e}");
                }
            }
        });

        handles.push(handle);

        // Small delay between connection attempts
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Wait for all clients to complete
    for handle in handles {
        let _ = handle.await;
    }

    println!("All clients finished");
}
