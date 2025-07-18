# How to Run a tace.chat Node

Running a `tace.chat` node is a lightweight and effective way to support the network's decentralization and censorship-resistance. This guide explains how to run a standalone node.

For production or scalable deployments, we suggest using Kubernetes. For a simpler setup on a single server, you can use Docker Compose.

## Prerequisites

- A publicly accessible server with a domain name pointing to it (e.g., `node.yourdomain.com`).
- [Docker](https://www.docker.com/get-started) and [Docker Compose](https://docs.docker.com/compose/install/) installed on your system if using the Docker-based setup.

## Option 1: Kubernetes

For scalable or resilient deployments, a good method is to run a `tace.chat` node on a Kubernetes cluster. We provide example manifests to help you get started.

The manifests for running a node are in the [k8s/](./k8s/) directory. Note that this directory also contains files for other components, like the `collector`, which are not needed for running a node.

## Option 2: Docker Compose & Traefik (Simple Setup)

This method provides a simple way to run a public-facing node on a single server. It uses Traefik as a reverse proxy to automatically provision and renew Let's Encrypt SSL certificates, enabling secure HTTPS for your node's API.

### 1. Create a `docker-compose.yml` file

Create a file named `docker-compose.yml` and paste the following content into it:

```yaml
version: '3.8'

services:
  tace-node:
    image: nootr/tace-node:latest # Use the official pre-built image
    container_name: tace-node
    restart: unless-stopped
    environment:
      - ADVERTISE_HOST=<YOUR_DOMAIN> # e.g., node.yourdomain.com
      - NODE_PORT=6512
      - API_PORT=80
    networks:
      - web
    labels:
      - "traefik.enable=true"
      - "traefik.http.routers.tace-node.rule=Host(`
`)" # e.g., Host(`node.yourdomain.com`)
      - "traefik.http.routers.tace-node.entrypoints=websecure"
      - "traefik.http.routers.tace-node.tls.certresolver=myresolver"
      - "traefik.http.services.tace-node.loadbalancer.server.port=80"
      # Also expose the P2P port via a TCP router
      - "traefik.tcp.routers.tace-p2p.rule=HostSNI(`*`)"
      - "traefik.tcp.routers.tace-p2p.entrypoints=tace-p2p"
      - "traefik.tcp.services.tace-p2p.loadbalancer.server.port=6512"

  traefik:
    image: "traefik:v2.5"
    container_name: "traefik"
    restart: unless-stopped
    command:
      - "--api.insecure=true"
      - "--providers.docker=true"
      - "--providers.docker.exposedbydefault=false"
      - "--entrypoints.web.address=:80"
      - "--entrypoints.websecure.address=:443"
      - "--entrypoints.tace-p2p.address=:6512" # Entrypoint for the P2P port
      - "--certificatesresolvers.myresolver.acme.httpchallenge=true"
      - "--certificatesresolvers.myresolver.acme.httpchallenge.entrypoint=web"
      - "--certificatesresolvers.myresolver.acme.email=<YOUR_EMAIL>" # e.g., you@yourdomain.com
      - "--certificatesresolvers.myresolver.acme.storage=/letsencrypt/acme.json"
    ports:
      - "80:80"
      - "443:443"
      - "6512:6512"
      - "8080:8080" # For Traefik dashboard
    volumes:
      - "/var/run/docker.sock:/var/run/docker.sock:ro"
      - "./letsencrypt:/letsencrypt"
    networks:
      - web

networks:
  web:
    driver: bridge
```

### 2. Configure and Run

1.  **Replace Placeholders**: In the `docker-compose.yml` file, replace `<YOUR_DOMAIN>` with your actual domain and `<YOUR_EMAIL>` with your email address for the SSL certificate.
2.  **Create Certificate Storage**: Create the file for storing the SSL certificate and set the correct permissions.
    ```bash
    mkdir letsencrypt
    touch letsencrypt/acme.json
    chmod 600 letsencrypt/acme.json
    ```
3.  **Start the Services**: Launch the node and Traefik.
    ```bash
    docker-compose up -d
    ```

Traefik will now handle incoming traffic, route it to your node, and automatically manage your SSL certificate.

## Verifying Your Node

You can check the logs to ensure your node started correctly and connected to the network. If you are using the Docker Compose setup, you can run:

```bash
docker logs tce-node
```

You should see log entries indicating that the node has started, joined the network, and is stabilizing.

```