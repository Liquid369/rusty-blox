#!/usr/bin/env python3
"""
PIVX Address Comparison Tool
Compares address data between Blockbook (reference) and local explorer.
Produces exact delta: missing/extra txids and UTXOs.
"""

import requests
import json
import sys
from typing import Dict, List, Set, Tuple
from collections import defaultdict

# Configuration
BLOCKBOOK_BASE = "https://explorer.duddino.com"
LOCAL_BASE = "http://localhost:3001"
ADDRESS = "DBh9o9uRGohcDKpeiEyRiwtmTaTL3xDdev"

# PIV has 8 decimals (1 PIV = 100000000 satoshi/duffs)
SATOSHI_PER_PIV = 100000000

def satoshi_to_piv(satoshi: int) -> float:
    """Convert satoshi/duffs to PIV"""
    return satoshi / SATOSHI_PER_PIV

def piv_to_satoshi(piv_str: str) -> int:
    """Convert PIV string to satoshi"""
    return int(float(piv_str) * SATOSHI_PER_PIV)

def fetch_blockbook_address(address: str) -> Dict:
    """Fetch address data from Blockbook API"""
    print(f"🔍 Fetching Blockbook data for {address}...")
    
    # Fetch full transaction list (no pagination needed for single call)
    url = f"{BLOCKBOOK_BASE}/api/v2/address/{address}?details=txs"
    print(f"   GET {url}")
    
    response = requests.get(url, timeout=60)
    response.raise_for_status()
    data = response.json()
    
    print(f"   ✅ Balance: {data.get('balance', 0)} sat")
    print(f"   ✅ Total Received: {data.get('totalReceived', 0)} sat")
    print(f"   ✅ Total Sent: {data.get('totalSent', 0)} sat")
    print(f"   ✅ Transactions: {data.get('txs', 0)}")
    
    return data

def fetch_blockbook_utxos(address: str) -> List[Dict]:
    """Fetch UTXOs from Blockbook"""
    print(f"\n🔍 Fetching Blockbook UTXOs for {address}...")
    
    url = f"{BLOCKBOOK_BASE}/api/v2/utxo/{address}"
    print(f"   GET {url}")
    
    response = requests.get(url, timeout=60)
    response.raise_for_status()
    utxos = response.json()
    
    print(f"   ✅ {len(utxos)} UTXOs")
    
    return utxos

def fetch_local_address(address: str) -> Dict:
    """Fetch address data from local API"""
    print(f"\n🔍 Fetching LOCAL data for {address}...")
    
    # Local API uses /api/v2/address/{address}
    url = f"{LOCAL_BASE}/api/v2/address/{address}?details=txids"
    print(f"   GET {url}")
    
    response = requests.get(url, timeout=60)
    response.raise_for_status()
    data = response.json()
    
    # Local API returns satoshi as strings, NOT PIV
    # No conversion needed - already in satoshi
    balance_sat = int(data.get('balance', '0'))
    received_sat = int(data.get('totalReceived', '0'))
    sent_sat = int(data.get('totalSent', '0'))
    
    print(f"   ✅ Balance: {balance_sat} sat ({satoshi_to_piv(balance_sat):.8f} PIV)")
    print(f"   ✅ Total Received: {received_sat} sat ({satoshi_to_piv(received_sat):.8f} PIV)")
    print(f"   ✅ Total Sent: {sent_sat} sat ({satoshi_to_piv(sent_sat):.8f} PIV)")
    print(f"   ✅ Transactions: {data.get('txs', 0)}")
    
    # Normalize to Blockbook format (satoshi strings)
    normalized = {
        'balance': str(balance_sat),
        'totalReceived': str(received_sat),
        'totalSent': str(sent_sat),
        'txs': data.get('txs', 0),
        'txids': data.get('txids', [])
    }
    
    return normalized

def fetch_local_utxos(address: str) -> List[Dict]:
    """Fetch UTXOs from local API"""
    print(f"\n🔍 Fetching LOCAL UTXOs for {address}...")
    
    url = f"{LOCAL_BASE}/api/v2/utxo/{address}"
    print(f"   GET {url}")
    
    response = requests.get(url, timeout=60)
    response.raise_for_status()
    utxos = response.json()
    
    # Local API returns satoshi as strings, NOT PIV
    # No conversion needed
    normalized_utxos = []
    for utxo in utxos:
        value_sat = int(utxo.get('value', '0'))
        normalized_utxos.append({
            'txid': utxo['txid'],
            'vout': utxo['vout'],
            'value': str(value_sat),
            'height': utxo.get('height'),
            'confirmations': utxo.get('confirmations', 0)
        })
    
    print(f"   ✅ {len(normalized_utxos)} UTXOs")
    
    return normalized_utxos

def normalize_txid(txid: str) -> str:
    """Normalize txid to lowercase"""
    return txid.lower()

def compute_tx_delta(blockbook_txids: List[str], local_txids: List[str]) -> Tuple[Set[str], Set[str]]:
    """Compute missing and extra transaction IDs"""
    bb_set = set(normalize_txid(txid) for txid in blockbook_txids)
    local_set = set(normalize_txid(txid) for txid in local_txids)
    
    missing = bb_set - local_set  # In Blockbook but not local
    extra = local_set - bb_set     # In local but not Blockbook
    
    return missing, extra

def compute_utxo_delta(blockbook_utxos: List[Dict], local_utxos: List[Dict]) -> Tuple[List[Dict], List[Dict]]:
    """Compute missing and extra UTXOs"""
    
    # Build outpoint maps: (txid, vout) -> utxo
    bb_map = {(normalize_txid(u['txid']), u['vout']): u for u in blockbook_utxos}
    local_map = {(normalize_txid(u['txid']), u['vout']): u for u in local_utxos}
    
    bb_outpoints = set(bb_map.keys())
    local_outpoints = set(local_map.keys())
    
    missing_outpoints = bb_outpoints - local_outpoints
    extra_outpoints = local_outpoints - bb_outpoints
    
    missing_utxos = [bb_map[op] for op in missing_outpoints]
    extra_utxos = [local_map[op] for op in extra_outpoints]
    
    return missing_utxos, extra_utxos

def fetch_transaction_details(txid: str, source: str = "blockbook") -> Dict:
    """Fetch full transaction details"""
    if source == "blockbook":
        url = f"{BLOCKBOOK_BASE}/api/v2/tx/{txid}"
    else:
        url = f"{LOCAL_BASE}/api/v2/tx/{txid}"
    
    try:
        response = requests.get(url, timeout=30)
        response.raise_for_status()
        return response.json()
    except Exception as e:
        print(f"   ⚠️  Failed to fetch tx {txid[:16]}... from {source}: {e}")
        return {}

def analyze_transaction_impact(txid: str, address: str, source: str = "blockbook") -> Dict:
    """Analyze how a transaction impacts address totals"""
    tx = fetch_transaction_details(txid, source)
    if not tx:
        return {}
    
    received = 0
    sent = 0
    
    # Check outputs for received amounts
    for vout in tx.get('vout', []):
        addresses = vout.get('addresses', [])
        if address in addresses:
            value_str = vout.get('value', '0')
            received += int(value_str) if value_str.isdigit() else piv_to_satoshi(value_str)
    
    # Check inputs for sent amounts
    for vin in tx.get('vin', []):
        addresses = vin.get('addresses', [])
        if address in addresses:
            value_str = vin.get('value', '0')
            if value_str:
                sent += int(value_str) if value_str.isdigit() else piv_to_satoshi(value_str)
    
    return {
        'txid': txid,
        'height': tx.get('blockHeight', -1),
        'received': received,
        'sent': sent,
        'net': received - sent
    }

def generate_report(address: str):
    """Generate complete comparison report"""
    print(f"\n{'='*80}")
    print(f"PIVX ADDRESS COMPARISON REPORT")
    print(f"Address: {address}")
    print(f"{'='*80}\n")
    
    # Fetch all data
    try:
        bb_address = fetch_blockbook_address(address)
        bb_utxos = fetch_blockbook_utxos(address)
        local_address = fetch_local_address(address)
        local_utxos = fetch_local_utxos(address)
    except Exception as e:
        print(f"\n❌ ERROR: Failed to fetch data: {e}")
        return
    
    # Extract transaction lists
    bb_txids = []
    if 'transactions' in bb_address:
        bb_txids = [tx['txid'] for tx in bb_address['transactions']]
    elif 'txids' in bb_address:
        bb_txids = bb_address['txids']
    
    local_txids = local_address.get('txids', [])
    
    print(f"\n{'='*80}")
    print(f"SUMMARY COMPARISON")
    print(f"{'='*80}\n")
    
    # Compare totals
    bb_balance = int(bb_address.get('balance', 0))
    bb_received = int(bb_address.get('totalReceived', 0))
    bb_sent = int(bb_address.get('totalSent', 0))
    bb_tx_count = int(bb_address.get('txs', 0))
    
    local_balance = int(local_address.get('balance', 0))
    local_received = int(local_address.get('totalReceived', 0))
    local_sent = int(local_address.get('totalSent', 0))
    local_tx_count = int(local_address.get('txs', 0))
    
    print(f"{'Metric':<25} {'Blockbook':<20} {'Local':<20} {'Delta':<20}")
    print(f"{'-'*85}")
    print(f"{'Balance (sat)':<25} {bb_balance:<20} {local_balance:<20} {local_balance - bb_balance:<20}")
    print(f"{'Balance (PIV)':<25} {satoshi_to_piv(bb_balance):<20.8f} {satoshi_to_piv(local_balance):<20.8f} {satoshi_to_piv(local_balance - bb_balance):<20.8f}")
    print(f"{'Total Received (sat)':<25} {bb_received:<20} {local_received:<20} {local_received - bb_received:<20}")
    print(f"{'Total Received (PIV)':<25} {satoshi_to_piv(bb_received):<20.8f} {satoshi_to_piv(local_received):<20.8f} {satoshi_to_piv(local_received - bb_received):<20.8f}")
    print(f"{'Total Sent (sat)':<25} {bb_sent:<20} {local_sent:<20} {local_sent - bb_sent:<20}")
    print(f"{'Total Sent (PIV)':<25} {satoshi_to_piv(bb_sent):<20.8f} {satoshi_to_piv(local_sent):<20.8f} {satoshi_to_piv(local_sent - bb_sent):<20.8f}")
    print(f"{'Transaction Count':<25} {bb_tx_count:<20} {local_tx_count:<20} {local_tx_count - bb_tx_count:<20}")
    
    # Compute deltas
    missing_txids, extra_txids = compute_tx_delta(bb_txids, local_txids)
    missing_utxos, extra_utxos = compute_utxo_delta(bb_utxos, local_utxos)
    
    print(f"\n{'='*80}")
    print(f"TRANSACTION DELTA")
    print(f"{'='*80}\n")
    print(f"Missing TXs (in Blockbook, not local): {len(missing_txids)}")
    print(f"Extra TXs (in local, not Blockbook):   {len(extra_txids)}")
    
    if missing_txids:
        print(f"\n📋 MISSING TRANSACTIONS:")
        for txid in sorted(missing_txids):
            print(f"   {txid}")
            impact = analyze_transaction_impact(txid, address, "blockbook")
            if impact:
                print(f"      Height: {impact['height']}")
                print(f"      Received: {satoshi_to_piv(impact['received']):.8f} PIV")
                print(f"      Sent: {satoshi_to_piv(impact['sent']):.8f} PIV")
                print(f"      Net: {satoshi_to_piv(impact['net']):.8f} PIV")
    
    if extra_txids:
        print(f"\n📋 EXTRA TRANSACTIONS (should not exist):")
        for txid in sorted(extra_txids):
            print(f"   {txid}")
    
    print(f"\n{'='*80}")
    print(f"UTXO DELTA")
    print(f"{'='*80}\n")
    print(f"Missing UTXOs (in Blockbook, not local): {len(missing_utxos)}")
    print(f"Extra UTXOs (in local, not Blockbook):   {len(extra_utxos)}")
    
    if missing_utxos:
        print(f"\n📋 MISSING UTXOS:")
        total_missing_value = 0
        for utxo in missing_utxos:
            value_sat = int(utxo['value'])
            total_missing_value += value_sat
            print(f"   {utxo['txid']}:{utxo['vout']}")
            print(f"      Value: {satoshi_to_piv(value_sat):.8f} PIV ({value_sat} sat)")
            print(f"      Height: {utxo.get('height', 'unknown')}")
            print(f"      Confirmations: {utxo.get('confirmations', 0)}")
        print(f"\n   💰 Total missing value: {satoshi_to_piv(total_missing_value):.8f} PIV")
    
    if extra_utxos:
        print(f"\n📋 EXTRA UTXOS (should not exist):")
        total_extra_value = 0
        for utxo in extra_utxos:
            value_sat = int(utxo['value'])
            total_extra_value += value_sat
            print(f"   {utxo['txid']}:{utxo['vout']}")
            print(f"      Value: {satoshi_to_piv(value_sat):.8f} PIV ({value_sat} sat)")
            print(f"      Height: {utxo.get('height', 'unknown')}")
        print(f"\n   💰 Total extra value: {satoshi_to_piv(total_extra_value):.8f} PIV")
    
    # Generate JSON report
    report = {
        'address': address,
        'blockbook': {
            'balance': bb_balance,
            'totalReceived': bb_received,
            'totalSent': bb_sent,
            'txCount': bb_tx_count,
            'utxoCount': len(bb_utxos)
        },
        'local': {
            'balance': local_balance,
            'totalReceived': local_received,
            'totalSent': local_sent,
            'txCount': local_tx_count,
            'utxoCount': len(local_utxos)
        },
        'delta': {
            'balance': local_balance - bb_balance,
            'totalReceived': local_received - bb_received,
            'totalSent': local_sent - bb_sent,
            'txCount': local_tx_count - bb_tx_count,
            'utxoCount': len(local_utxos) - len(bb_utxos)
        },
        'missing_txids': sorted(list(missing_txids)),
        'extra_txids': sorted(list(extra_txids)),
        'missing_utxos': missing_utxos,
        'extra_utxos': extra_utxos
    }
    
    # Save JSON report
    json_file = f"diff_{address[:10]}.json"
    with open(json_file, 'w') as f:
        json.dump(report, f, indent=2)
    print(f"\n✅ JSON report saved to: {json_file}")
    
    print(f"\n{'='*80}")
    print(f"ANALYSIS COMPLETE")
    print(f"{'='*80}\n")

if __name__ == "__main__":
    generate_report(ADDRESS)
