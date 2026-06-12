#!/bin/bash
# Install RustyBlox as a systemd service

set -e

echo "🔧 Installing RustyBlox systemd service..."

# Copy service file
sudo cp rustyblox.service /etc/systemd/system/

# Reload systemd
sudo systemctl daemon-reload

# Enable service to start on boot
sudo systemctl enable rustyblox

echo "✅ Service installed!"
echo ""
echo "Available commands:"
echo "  sudo systemctl start rustyblox    # Start the service"
echo "  sudo systemctl stop rustyblox     # Stop the service"
echo "  sudo systemctl restart rustyblox  # Restart the service"
echo "  sudo systemctl status rustyblox   # Check status"
echo "  sudo journalctl -u rustyblox -f   # View logs"
echo ""
echo "To start now, run:"
echo "  sudo systemctl start rustyblox"
