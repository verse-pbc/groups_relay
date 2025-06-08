#!/usr/bin/env python3

"""
Test WebSocket connection establishment performance for Nostr relay.
Measures connection timing, handshake duration, and first message latency.
"""

import asyncio
import json
import time
import statistics
import argparse
import sys
from typing import List, Dict, Tuple
from datetime import datetime

try:
    import websockets
except ImportError:
    print("Error: websockets library not found. Install with: pip install websockets")
    sys.exit(1)


class ConnectionTester:
    def __init__(self, relay_url: str, verbose: bool = False):
        self.relay_url = relay_url
        self.verbose = verbose
        self.results = {
            'connection_times': [],
            'handshake_times': [],
            'first_message_times': [],
            'total_times': [],
            'failures': 0,
            'connection_errors': [],
            'timeouts': 0
        }
    
    async def test_single_connection(self, index: int) -> Dict[str, float]:
        """Test a single WebSocket connection and measure timings."""
        times = {}
        
        # Start timing
        start_time = time.perf_counter()
        
        try:
            # Connection establishment
            connection_start = time.perf_counter()
            websocket = await asyncio.wait_for(
                websockets.connect(self.relay_url),
                timeout=10.0
            )
            connection_end = time.perf_counter()
            times['connection'] = (connection_end - connection_start) * 1000
            
            # Send initial REQ message (simulate real client behavior)
            handshake_start = time.perf_counter()
            req_message = json.dumps([
                "REQ",
                f"test-{index}",
                {"kinds": [1], "limit": 1}
            ])
            await websocket.send(req_message)
            handshake_end = time.perf_counter()
            times['handshake'] = (handshake_end - handshake_start) * 1000
            
            # Wait for first response
            first_msg_start = time.perf_counter()
            response = await asyncio.wait_for(websocket.recv(), timeout=5.0)
            first_msg_end = time.perf_counter()
            times['first_message'] = (first_msg_end - first_msg_start) * 1000
            
            # Close connection
            await websocket.close()
            
            # Total time
            end_time = time.perf_counter()
            times['total'] = (end_time - start_time) * 1000
            
            if self.verbose:
                print(f"Connection {index}: Success - "
                      f"Connect: {times['connection']:.2f}ms, "
                      f"Handshake: {times['handshake']:.2f}ms, "
                      f"First msg: {times['first_message']:.2f}ms")
            
            return times
            
        except asyncio.TimeoutError:
            self.results['timeouts'] += 1
            if self.verbose:
                print(f"Connection {index}: Timeout")
            return None
            
        except Exception as e:
            self.results['failures'] += 1
            error_msg = str(e)
            if error_msg not in self.results['connection_errors']:
                self.results['connection_errors'].append(error_msg)
            if self.verbose:
                print(f"Connection {index}: Failed - {error_msg}")
            return None
    
    async def test_sequential_connections(self, count: int, delay_ms: int = 100):
        """Test connections sequentially with optional delay."""
        print(f"\nTesting {count} sequential connections...")
        
        for i in range(count):
            times = await self.test_single_connection(i)
            if times:
                self.results['connection_times'].append(times['connection'])
                self.results['handshake_times'].append(times['handshake'])
                self.results['first_message_times'].append(times['first_message'])
                self.results['total_times'].append(times['total'])
            
            # Progress indicator
            if not self.verbose and (i + 1) % 10 == 0:
                print(f"  Progress: {i + 1}/{count}")
            
            # Delay between connections
            if i < count - 1 and delay_ms > 0:
                await asyncio.sleep(delay_ms / 1000)
    
    async def test_parallel_connections(self, count: int, batch_size: int = 10):
        """Test connections in parallel batches."""
        print(f"\nTesting {count} parallel connections (batch size: {batch_size})...")
        
        for batch_start in range(0, count, batch_size):
            batch_end = min(batch_start + batch_size, count)
            batch_tasks = []
            
            for i in range(batch_start, batch_end):
                batch_tasks.append(self.test_single_connection(i))
            
            # Run batch in parallel
            results = await asyncio.gather(*batch_tasks, return_exceptions=True)
            
            # Process results
            for times in results:
                if isinstance(times, dict):
                    self.results['connection_times'].append(times['connection'])
                    self.results['handshake_times'].append(times['handshake'])
                    self.results['first_message_times'].append(times['first_message'])
                    self.results['total_times'].append(times['total'])
            
            # Progress indicator
            if not self.verbose:
                print(f"  Progress: {batch_end}/{count}")
    
    def print_statistics(self):
        """Print detailed statistics of the test results."""
        print("\n" + "="*60)
        print(f"CONNECTION PERFORMANCE RESULTS - {self.relay_url}")
        print("="*60)
        
        total_attempts = len(self.results['connection_times']) + self.results['failures'] + self.results['timeouts']
        success_rate = (len(self.results['connection_times']) / total_attempts * 100) if total_attempts > 0 else 0
        
        print(f"\nSummary:")
        print(f"  Total attempts: {total_attempts}")
        print(f"  Successful: {len(self.results['connection_times'])}")
        print(f"  Failed: {self.results['failures']}")
        print(f"  Timeouts: {self.results['timeouts']}")
        print(f"  Success rate: {success_rate:.1f}%")
        
        if self.results['connection_errors']:
            print(f"\nUnique errors encountered:")
            for error in self.results['connection_errors']:
                print(f"  - {error}")
        
        # Connection establishment statistics
        if self.results['connection_times']:
            print(f"\nConnection Establishment Times (ms):")
            self._print_timing_stats(self.results['connection_times'])
            
            print(f"\nHandshake Times (ms):")
            self._print_timing_stats(self.results['handshake_times'])
            
            print(f"\nFirst Message Times (ms):")
            self._print_timing_stats(self.results['first_message_times'])
            
            print(f"\nTotal Operation Times (ms):")
            self._print_timing_stats(self.results['total_times'])
            
            # Distribution analysis
            print(f"\nConnection Time Distribution:")
            self._print_distribution(self.results['connection_times'])
    
    def _print_timing_stats(self, times: List[float]):
        """Print timing statistics for a list of measurements."""
        if not times:
            print("  No data available")
            return
        
        sorted_times = sorted(times)
        print(f"  Min:    {min(times):.2f}")
        print(f"  Max:    {max(times):.2f}")
        print(f"  Mean:   {statistics.mean(times):.2f}")
        print(f"  Median: {statistics.median(times):.2f}")
        if len(times) > 1:
            print(f"  StdDev: {statistics.stdev(times):.2f}")
        print(f"  P50:    {sorted_times[int(len(sorted_times) * 0.50)]:.2f}")
        print(f"  P90:    {sorted_times[int(len(sorted_times) * 0.90)]:.2f}")
        print(f"  P95:    {sorted_times[int(len(sorted_times) * 0.95)]:.2f}")
        print(f"  P99:    {sorted_times[int(len(sorted_times) * 0.99)]:.2f}")
    
    def _print_distribution(self, times: List[float]):
        """Print a simple distribution of timing values."""
        if not times:
            return
        
        # Create buckets
        min_time = min(times)
        max_time = max(times)
        bucket_count = 10
        bucket_size = (max_time - min_time) / bucket_count
        
        buckets = [0] * bucket_count
        for t in times:
            bucket_idx = min(int((t - min_time) / bucket_size), bucket_count - 1)
            buckets[bucket_idx] += 1
        
        # Print histogram
        max_count = max(buckets)
        for i, count in enumerate(buckets):
            start = min_time + i * bucket_size
            end = start + bucket_size
            bar_length = int((count / max_count) * 40) if max_count > 0 else 0
            bar = "█" * bar_length
            print(f"  {start:6.1f}-{end:6.1f}ms: {bar} ({count})")
    
    def save_results(self, filename: str):
        """Save raw results to a JSON file for later analysis."""
        output = {
            'timestamp': datetime.now().isoformat(),
            'relay_url': self.relay_url,
            'statistics': {
                'total_attempts': len(self.results['connection_times']) + self.results['failures'] + self.results['timeouts'],
                'successful': len(self.results['connection_times']),
                'failed': self.results['failures'],
                'timeouts': self.results['timeouts']
            },
            'timings': {
                'connection_times': self.results['connection_times'],
                'handshake_times': self.results['handshake_times'],
                'first_message_times': self.results['first_message_times'],
                'total_times': self.results['total_times']
            },
            'errors': self.results['connection_errors']
        }
        
        with open(filename, 'w') as f:
            json.dump(output, f, indent=2)
        print(f"\nResults saved to: {filename}")


async def main():
    parser = argparse.ArgumentParser(description='Test WebSocket connection performance for Nostr relay')
    parser.add_argument('relay_url', nargs='?', default='ws://localhost:8080',
                        help='WebSocket URL of the relay (default: ws://localhost:8080)')
    parser.add_argument('-n', '--count', type=int, default=100,
                        help='Number of connections to test (default: 100)')
    parser.add_argument('-m', '--mode', choices=['sequential', 'parallel', 'both'], default='both',
                        help='Test mode (default: both)')
    parser.add_argument('-d', '--delay', type=int, default=50,
                        help='Delay between sequential connections in ms (default: 50)')
    parser.add_argument('-b', '--batch-size', type=int, default=10,
                        help='Batch size for parallel connections (default: 10)')
    parser.add_argument('-v', '--verbose', action='store_true',
                        help='Verbose output')
    parser.add_argument('-o', '--output', type=str,
                        help='Save results to JSON file')
    
    args = parser.parse_args()
    
    print(f"Nostr Relay Connection Performance Tester")
    print(f"Target: {args.relay_url}")
    print(f"Test count: {args.count}")
    print(f"Mode: {args.mode}")
    
    # Run tests based on mode
    if args.mode in ['sequential', 'both']:
        tester = ConnectionTester(args.relay_url, args.verbose)
        await tester.test_sequential_connections(args.count, args.delay)
        tester.print_statistics()
        if args.output:
            tester.save_results(f"{args.output}_sequential.json")
    
    if args.mode in ['parallel', 'both']:
        tester = ConnectionTester(args.relay_url, args.verbose)
        await tester.test_parallel_connections(args.count, args.batch_size)
        tester.print_statistics()
        if args.output:
            tester.save_results(f"{args.output}_parallel.json")
    
    print("\nTest completed!")


if __name__ == "__main__":
    asyncio.run(main())