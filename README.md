# Strim

## Exersize project:
### A pair of CLI applications - a client and a server - for capturing audio data.

## Strim_server

Captures default device's audio
Creates a TCP server and awaits for connection.
Upon connection, starts streaming the audio to the connected client

## Strim_client

Connects to the Strim_server (so far connection is limited to localhost:8080)
Upon connection, plays received audio data into the default output audio device.

This is work in progress, don't use for anything serious!
