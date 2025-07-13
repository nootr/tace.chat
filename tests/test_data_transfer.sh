#!/bin/bash

# Test script for data transfer between nodes when new nodes join

echo "Starting test for data transfer between nodes..."

# Kill any existing nodes and processes using our ports
pkill -f tace_node
lsof -ti:5001 -ti:5002 -ti:6001 -ti:6002 | xargs kill -9 2>/dev/null
sleep 1

# Start first node (bootstrap)
echo "Starting bootstrap node on port 5001..."
cd node && RUST_LOG=info NODE_ADDRESS=127.0.0.1:5001 API_PORT=6001 cargo run > /tmp/node1.log 2>&1 &
NODE1_PID=$!
sleep 5

# Send a test message to node 1
echo "Sending test message to node 1..."
curl -X POST http://localhost:6001/message \
  -H "Content-Type: application/json" \
  -d '{
    "to": "test_recipient_public_key",
    "from": "test_sender_public_key",
    "ciphertext": [72, 101, 108, 108, 111],
    "nonce": [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]
  }'

sleep 1

# Start second node joining the network
echo "Starting second node on port 5002, joining network..."
cd node && RUST_LOG=info NODE_ADDRESS=127.0.0.1:5002 API_PORT=6002 BOOTSTRAP_ADDRESS=127.0.0.1:5001 cargo run > /tmp/node2.log 2>&1 &
NODE2_PID=$!
sleep 3

# Check logs to see if data transfer occurred
echo "Checking logs for data transfer..."
echo "Node 1 log:"
grep -E "(Transferring|Retrieved|Stored)" /tmp/node1.log | tail -10
echo ""
echo "Node 2 log:"
grep -E "(Requesting data|Received|Stored)" /tmp/node2.log | tail -10

# Cleanup
echo ""
echo "Cleaning up..."
kill $NODE1_PID $NODE2_PID 2>/dev/null

echo "Test complete! Check /tmp/node*.log for full logs."
