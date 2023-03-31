#!/bin/bash
# Disable a coin using ./disable_coin.sh coin_ticker
# Example: `./disble_coin.sh KMD` to disable KMD

source userpass
curl --url "http://127.0.0.1:7783" --data "{\"userpass\":\"$userpass\",\"method\":\"disable_coin\",\"coin\":\"$1\"}"
