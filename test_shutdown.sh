#!/bin/bash

echo "Testing client graceful shutdown..."

# Start the unified client in the background (defaults: host localhost, port DEFAULT_PORT)
cargo run -p strim -- client &
CLIENT_PID=$!

echo "Client started with PID: $CLIENT_PID"
echo "Waiting 3 seconds to let it try to connect..."
sleep 3

echo "Sending SIGINT (Ctrl+C) to client..."
kill -INT $CLIENT_PID

echo "Waiting up to 5 seconds for client to terminate..."
for i in {1..50}; do
    if ! kill -0 $CLIENT_PID 2>/dev/null; then
        echo "Client terminated gracefully after $(($i * 100))ms"
        exit 0
    fi
    sleep 0.1
done

echo "ERROR: Client did not terminate within 5 seconds!"
echo "Forcefully killing client..."
kill -9 $CLIENT_PID 2>/dev/null
exit 1