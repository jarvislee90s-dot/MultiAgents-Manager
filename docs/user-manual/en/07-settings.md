# Settings

Notifications, polling, tool detection settings

## Overview

This chapter covers Settings functionality in MultiAgents Manager.

## Features

MultiAgents Manager provides comprehensive Settings features for managing AI programming tools.

## Remote Access

Once Remote Access is enabled in Settings, the session dashboard on this machine can be opened from other devices' browsers through four channels — LAN, quick tunnel, own domain, and external domain (no domain needed) — all protected by a 4-digit access password.

**The browser on this computer also needs the access password**: opening the dashboard from this machine's own browser is treated exactly like a remote device — enter the access password once on first visit, and it is then remembered for 180 days.

### External Domain (No Domain Needed) Channel

Even without your own domain, you can get a **permanent, never-changing** access address:

1. Install [Tailscale](https://tailscale.com/) on this machine and sign in once (browser authorization is enough)
2. Choose "External domain (no domain needed)" in the Remote Access channel area and follow the guide
3. Once enabled you get an address like `https://<machine>.<tailnet>.ts.net/m` — it never changes and works from any network

The app verifies the address is reachable first and only shows it as available after that; once active it re-checks every minute and tries to recover automatically on failure.

**Bandwidth limit (a known property of this channel, not a malfunction)**: this external channel is bandwidth-limited (measured roughly 1.8 Mbps down / 0.8 Mbps up) — downloading a 20 MB attachment takes about 1.5 minutes and uploading from the phone about 3.3 minutes, so large files are noticeably slow. For full speed, install Tailscale on the phone too and connect directly over the tailnet.

## Steps

1. Open the main application window
2. Configure relevant options
3. Confirm settings are applied

## Notes

- Read the quick start guide for first-time setup
- Refer to troubleshooting for issues
- Visit the GitHub repository for more information

![Screenshot](../../assets/screenshots/en/07-settings.png)
