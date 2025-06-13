#!/usr/bin/env node

const WebSocket = require('ws');
const https = require('https');
const http = require('http');
const tls = require('tls');
const { URL } = require('url');
const { performance } = require('perf_hooks');
const dns = require('dns').promises;

// ANSI color codes
const colors = {
  reset: '\x1b[0m',
  bright: '\x1b[1m',
  dim: '\x1b[2m',
  red: '\x1b[31m',
  green: '\x1b[32m',
  yellow: '\x1b[33m',
  blue: '\x1b[34m',
  cyan: '\x1b[36m',
  gray: '\x1b[90m'
};

class NetworkDiagnostics {
  constructor(url, options = {}) {
    this.url = url;
    this.parsedUrl = new URL(url);
    this.hostname = this.parsedUrl.hostname;
    this.port = this.parsedUrl.port || (this.parsedUrl.protocol === 'wss:' ? 443 : 80);
    this.isSecure = this.parsedUrl.protocol === 'wss:';
    
    this.options = {
      iterations: options.iterations || 5,
      mode: options.mode || 'all',
      verbose: options.verbose || false,
      timeout: options.timeout || 10000
    };
  }

  log(message, color = '') {
    console.log(color + message + colors.reset);
  }

  async measureSimpleRTT() {
    this.log('\n=== Simple WebSocket RTT Test ===', colors.bright);
    this.log(`Testing ${this.url} with ${this.options.iterations} iterations...`, colors.gray);
    
    const results = [];
    
    for (let i = 0; i < this.options.iterations; i++) {
      const startTime = Date.now();
      
      try {
        const ws = new WebSocket(this.url);
        
        await new Promise((resolve, reject) => {
          ws.on('open', () => {
            const rtt = Date.now() - startTime;
            results.push(rtt);
            this.log(`  Iteration ${i + 1}: ${rtt}ms`, colors.green);
            ws.close();
            resolve();
          });
          
          ws.on('error', (error) => {
            this.log(`  Iteration ${i + 1}: Error - ${error.message}`, colors.red);
            reject(error);
          });
          
          setTimeout(() => {
            ws.close();
            reject(new Error('Connection timeout'));
          }, this.options.timeout);
        });
        
        await new Promise(resolve => setTimeout(resolve, 100));
        
      } catch (error) {
        this.log(`  Failed iteration ${i + 1}: ${error.message}`, colors.red);
      }
    }
    
    this.printStatistics('Simple RTT', results);
    return results;
  }

  async measureTCPLatency() {
    const startTime = Date.now();
    
    return new Promise((resolve) => {
      const options = {
        hostname: this.hostname,
        port: this.port,
        method: 'HEAD',
        timeout: 5000
      };
      
      const proto = this.isSecure ? https : http;
      const req = proto.request(options, (res) => {
        const tcpTime = Date.now() - startTime;
        res.on('data', () => {});
        res.on('end', () => {
          resolve(tcpTime);
        });
      });
      
      req.on('error', (error) => {
        if (this.options.verbose) {
          this.log(`TCP measurement error: ${error.message}`, colors.red);
        }
        resolve(-1);
      });
      
      req.end();
    });
  }

  async measureDetailedRTT() {
    this.log('\n=== Detailed WebSocket Analysis ===', colors.bright);
    this.log(`Host: ${this.hostname}, Port: ${this.port}, Secure: ${this.isSecure}`, colors.gray);
    
    // TCP/HTTP latency
    this.log('\n1. TCP/HTTP Connection Latency:', colors.cyan);
    const tcpLatencies = [];
    for (let i = 0; i < this.options.iterations; i++) {
      const tcpTime = await this.measureTCPLatency();
      if (tcpTime > 0) {
        tcpLatencies.push(tcpTime);
        this.log(`   Iteration ${i + 1}: ${tcpTime}ms`);
      }
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    
    // Full WebSocket connection
    this.log('\n2. Full WebSocket Connection RTT:', colors.cyan);
    const wsLatencies = [];
    const upgradeLatencies = [];
    
    for (let i = 0; i < this.options.iterations; i++) {
      const startTime = Date.now();
      let upgradeTime = 0;
      
      try {
        const ws = new WebSocket(this.url);
        
        ws.on('upgrade', () => {
          upgradeTime = Date.now() - startTime;
        });
        
        await new Promise((resolve, reject) => {
          ws.on('open', () => {
            const totalTime = Date.now() - startTime;
            wsLatencies.push(totalTime);
            if (upgradeTime > 0) {
              upgradeLatencies.push(upgradeTime);
            }
            this.log(`   Iteration ${i + 1}: Total=${totalTime}ms${upgradeTime > 0 ? `, Upgrade=${upgradeTime}ms` : ''}`);
            ws.close();
            resolve();
          });
          
          ws.on('error', reject);
          
          setTimeout(() => {
            ws.close();
            reject(new Error('Connection timeout'));
          }, this.options.timeout);
        });
        
        await new Promise(resolve => setTimeout(resolve, 100));
        
      } catch (error) {
        this.log(`   Iteration ${i + 1}: Error - ${error.message}`, colors.red);
      }
    }
    
    // Summary
    this.log('\n3. Connection Analysis:', colors.cyan);
    if (tcpLatencies.length > 0 && wsLatencies.length > 0) {
      const avgTcp = tcpLatencies.reduce((a, b) => a + b, 0) / tcpLatencies.length;
      const avgWs = wsLatencies.reduce((a, b) => a + b, 0) / wsLatencies.length;
      const overhead = avgWs - avgTcp;
      
      this.log(`   TCP/HTTP Average: ${avgTcp.toFixed(2)}ms`);
      this.log(`   WebSocket Average: ${avgWs.toFixed(2)}ms`);
      this.log(`   WebSocket Overhead: ${overhead.toFixed(2)}ms (${((overhead/avgWs)*100).toFixed(1)}% of total)`);
    }
    
    return { tcp: tcpLatencies, websocket: wsLatencies, upgrade: upgradeLatencies };
  }

  async measureSSLHandshake() {
    if (!this.isSecure) {
      this.log('\n=== SSL/TLS Analysis ===', colors.bright);
      this.log('Skipped: Not a secure connection', colors.yellow);
      return null;
    }
    
    this.log('\n=== SSL/TLS Handshake Analysis ===', colors.bright);
    
    const results = {
      dns: [],
      tcp: [],
      ssl: [],
      total: []
    };
    
    for (let i = 0; i < this.options.iterations; i++) {
      const startTotal = performance.now();
      
      try {
        // DNS lookup
        const startDns = performance.now();
        const { address } = await dns.lookup(this.hostname);
        const dnsTime = performance.now() - startDns;
        results.dns.push(dnsTime);
        
        // SSL handshake
        await new Promise((resolve, reject) => {
          const startTcp = performance.now();
          
          const socket = tls.connect({
            host: this.hostname,
            port: this.port,
            servername: this.hostname,
            rejectUnauthorized: true
          }, () => {
            const totalTime = performance.now() - startTotal;
            const tcpSslTime = performance.now() - startTcp;
            
            results.tcp.push(tcpSslTime - dnsTime);
            results.ssl.push(tcpSslTime);
            results.total.push(totalTime);
            
            this.log(`Iteration ${i + 1}:`);
            this.log(`  DNS: ${dnsTime.toFixed(2)}ms`);
            this.log(`  TCP+SSL: ${tcpSslTime.toFixed(2)}ms`);
            this.log(`  Total: ${totalTime.toFixed(2)}ms`);
            
            if (this.options.verbose) {
              this.log(`  TLS Version: ${socket.getProtocol()}`, colors.gray);
              this.log(`  Cipher: ${socket.getCipher().name}`, colors.gray);
            }
            
            socket.end();
            resolve();
          });
          
          socket.on('error', reject);
        });
        
        await new Promise(resolve => setTimeout(resolve, 200));
        
      } catch (error) {
        this.log(`Iteration ${i + 1} failed: ${error.message}`, colors.red);
      }
    }
    
    if (results.total.length > 0) {
      this.log('\nSSL/TLS Summary:', colors.cyan);
      this.log(`  DNS Average: ${(results.dns.reduce((a,b) => a+b, 0) / results.dns.length).toFixed(2)}ms`);
      this.log(`  TCP+SSL Average: ${(results.ssl.reduce((a,b) => a+b, 0) / results.ssl.length).toFixed(2)}ms`);
      this.log(`  Total Average: ${(results.total.reduce((a,b) => a+b, 0) / results.total.length).toFixed(2)}ms`);
    }
    
    return results;
  }

  async testSessionResumption() {
    if (!this.isSecure) {
      return null;
    }
    
    this.log('\n=== TLS Session Resumption Test ===', colors.bright);
    
    let sessionData = null;
    const times = [];
    
    for (let i = 0; i < 5; i++) {
      const start = performance.now();
      
      await new Promise((resolve, reject) => {
        const options = {
          host: this.hostname,
          port: this.port,
          servername: this.hostname,
          session: sessionData
        };
        
        const socket = tls.connect(options, () => {
          const time = performance.now() - start;
          times.push(time);
          
          const resumed = socket.isSessionReused();
          this.log(`Connection ${i + 1}: ${time.toFixed(2)}ms (Session ${resumed ? 'RESUMED' : 'NEW'})`, 
                   resumed ? colors.green : colors.yellow);
          
          sessionData = socket.getSession();
          
          socket.end();
          resolve();
        });
        
        socket.on('error', reject);
      });
      
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    
    if (times.length > 1) {
      this.log(`\nFirst connection: ${times[0].toFixed(2)}ms`);
      this.log(`Resumed avg: ${(times.slice(1).reduce((a,b) => a+b, 0) / (times.length-1)).toFixed(2)}ms`);
      this.log(`Improvement: ${(times[0] - times[times.length-1]).toFixed(2)}ms`, colors.green);
    }
    
    return times;
  }

  printStatistics(label, results) {
    if (results.length === 0) {
      this.log(`\nNo successful ${label} measurements!`, colors.red);
      return;
    }
    
    const avg = results.reduce((a, b) => a + b, 0) / results.length;
    const min = Math.min(...results);
    const max = Math.max(...results);
    const sorted = [...results].sort((a, b) => a - b);
    const median = sorted[Math.floor(sorted.length / 2)];
    
    this.log(`\n${label} Statistics:`, colors.cyan);
    this.log(`  Successful: ${results.length}/${this.options.iterations}`);
    this.log(`  Average: ${avg.toFixed(2)}ms`);
    this.log(`  Minimum: ${min}ms`);
    this.log(`  Maximum: ${max}ms`);
    this.log(`  Median: ${median}ms`);
  }

  async run() {
    this.log(`\n${colors.bright}Network Diagnostics for ${this.url}${colors.reset}`);
    this.log('=' + '='.repeat(59));
    
    const startTime = Date.now();
    
    try {
      switch (this.options.mode) {
        case 'simple':
          await this.measureSimpleRTT();
          break;
          
        case 'detailed':
          await this.measureDetailedRTT();
          break;
          
        case 'ssl':
          await this.measureSSLHandshake();
          await this.testSessionResumption();
          break;
          
        case 'all':
        default:
          await this.measureSimpleRTT();
          await this.measureDetailedRTT();
          await this.measureSSLHandshake();
          await this.testSessionResumption();
          break;
      }
      
      const totalTime = ((Date.now() - startTime) / 1000).toFixed(1);
      this.log(`\n${colors.bright}Total test time: ${totalTime}s${colors.reset}`);
      
    } catch (error) {
      this.log(`\nTest failed: ${error.message}`, colors.red);
      process.exit(1);
    }
  }
}

// CLI interface
function printUsage() {
  console.log(`
${colors.bright}Network Latency Diagnostics Tool${colors.reset}

Usage: node test_network_latency.js [URL] [OPTIONS]

Options:
  --mode, -m       Test mode: all, simple, detailed, ssl (default: all)
  --iterations, -i Number of iterations per test (default: 5)
  --verbose, -v    Show additional details
  --timeout, -t    Connection timeout in ms (default: 10000)
  --help, -h       Show this help message

Examples:
  node test_network_latency.js ws://localhost:3033
  node test_network_latency.js wss://relay.example.com -m simple -i 10
  node test_network_latency.js wss://relay.example.com --mode=ssl --verbose
`);
}

// Parse command line arguments
function parseArgs(args) {
  const options = {
    url: null,
    iterations: 5,
    mode: 'all',
    verbose: false,
    timeout: 10000
  };
  
  for (let i = 2; i < args.length; i++) {
    const arg = args[i];
    
    if (arg === '--help' || arg === '-h') {
      printUsage();
      process.exit(0);
    }
    
    if (arg.startsWith('--') || arg.startsWith('-')) {
      const [key, value] = arg.includes('=') ? arg.split('=') : [arg, args[++i]];
      
      switch (key) {
        case '--mode':
        case '-m':
          options.mode = value;
          break;
        case '--iterations':
        case '-i':
          options.iterations = parseInt(value) || 5;
          break;
        case '--verbose':
        case '-v':
          options.verbose = true;
          if (value && value.startsWith('-')) i--; // No value provided
          break;
        case '--timeout':
        case '-t':
          options.timeout = parseInt(value) || 10000;
          break;
      }
    } else if (!options.url) {
      options.url = arg;
    }
  }
  
  return options;
}

// Main
async function main() {
  const args = parseArgs(process.argv);
  
  if (!args.url) {
    console.error(`${colors.red}Error: URL is required${colors.reset}`);
    printUsage();
    process.exit(1);
  }
  
  try {
    new URL(args.url); // Validate URL
  } catch (error) {
    console.error(`${colors.red}Error: Invalid URL format${colors.reset}`);
    process.exit(1);
  }
  
  const diagnostics = new NetworkDiagnostics(args.url, args);
  await diagnostics.run();
}

main().catch(console.error);