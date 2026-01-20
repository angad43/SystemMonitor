# System Monitor App

A high-performance, native desktop system monitor built with **Rust** and **Slint**. This project was created as a deep-dive into hardware-level data collection, native UI rendering, and the Rust ecosystem.

![Rust](https://img.shields.io/badge/language-Rust-orange)
![UI](https://img.shields.io/badge/framework-Slint-green)

## Overview

System Monitor provides real-time insights into your computer's performance. Using Slint for a sleek UI and Rust for performance 

## The Purpose 

This project is as a learning experience to master several core concepts:

1.  **Slint:** Using Slint to create a somewhat modern and hardware-accelerated UI 
2.  **Low Level in Rust:** Using Rust to interface with system APIs.

## TODO
1. Add a page for memory, network, wifi and gpu
2. Make the app size dynamic
3. Optimize the app  
## Installation & Setup

### Prerequisites
* **Rust & Cargo:** [Install Rust](https://www.rust-lang.org/tools/install)
* **Dependencies (Arch):**
    ```bash
    sudo pacman -S fontconfig pkgconf
    ```

### Build from Source
1. Clone the repository:
   ```bash
   git clone https://github.com/angad43/SystemMonitor
   cd SystemMonitor   ```
2. Build and run:
   ```bash
   cargo run --release
   ```


## Examples
### CPU
<img width="984" height="785" alt="Screenshot2026-01-20 12-40-22" src="https://github.com/user-attachments/assets/3c368913-46ce-4aac-a868-4081829ecc48" />
### Memory
<img width="988" height="787" alt="Screenshot2026-01-20 20-50-45" src="https://github.com/user-attachments/assets/392d9cc4-6a51-40fe-a4a2-7f056f334a86" />
