#!/bin/bash

size=$((300 * 1024 * 1024)) # 300MB
echo "Allocating ${size} bytes (~300MB)..."

data=$(head -c $size /dev/zero | tr '\0' 'A')

echo "Memory allocated. PID=$$"
