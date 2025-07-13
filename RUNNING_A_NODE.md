# How to Run a tace.chat Node

Running a `tace.chat` node is a lightweight and effective way to support the network's decentralization and censorship-resistance. This guide explains how to run a standalone node using Docker.

## Prerequisites

- [Docker](https://www.docker.com/get-started) installed on your system.
- A publicly accessible IP address or domain name.

## System Requirements

The `tace.chat` node is designed to be lightweight.

-   **Memory**: 256MB of RAM is recommended. The node itself uses a small amount of memory, with a default 100MB in-memory queue for messages.
-   **CPU**: A single CPU core is sufficient.
-   **Disk Space**: Less than 200MB for the Docker image and log files.
-   **Network**: A stable internet connection with a public IP address and open ports for P2P and API communication.

## Running a Standalone Node with Docker

These instructions are for running a single, public-facing node that can join the main `tace.chat` network.

### 1. Build the Docker Image

First, clone the repository and build the Docker image for the node.

```bash
git clone https://github.com/nootr/tace.chat.git
cd tace.chat
docker build -t tace-node -f Dockerfile.node .
```

### 2. Run the Node

To run the node, you need to configure its public address and expose the correct ports. The node uses the following default ports:

-   **`8000/tcp`**: For P2P communication with other nodes.
-   **`8001/tcp`**: For the client API.

You must provide the public-facing address of your node via the `NODE_ADDRESS` environment variable. You may also need to specify a `BOOTSTRAP_ADDRESS` to join the public network.

```bash
docker run -d \
  -p 8000:8000/tcp \
  -p 8001:8001/tcp \
  -e NODE_ADDRESS="your_public_domain_or_ip:8000" \
  -e BOOTSTRAP_ADDRESS="bootstrap.tace.chat:8000" \
  --name tace-node \
  tace-node
```

**Replace the following values:**

-   `your_public_domain_or_ip:8000`: The public address and P2P port where your node can be reached.
-   `bootstrap.tace.chat:8000`: The address of a well-known node to join the network. (Note: This is an example address).

### Example

If your server's IP is `203.0.113.42`, the command would be:

```bash
docker run -d \
  -p 8000:8000/tcp \
  -p 8001:8001/tcp \
  -e NODE_ADDRESS="203.0.113.42:8000" \
  -e BOOTSTRAP_ADDRESS="bootstrap.tace.chat:8000" \
  --name tace-node \
  tace-node
```

### 3. Verifying Your Node

You can check the logs to ensure your node started correctly and connected to the network.

```bash
docker logs tace-node
```

You should see log entries indicating that the node has started, joined the network, and is stabilizing.