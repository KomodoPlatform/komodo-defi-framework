#!/bin/bash
for ((i=2;i<256;i++))
do
    sudo ifconfig lo0 alias 127.0.0.$i up
done
sudo ifconfig lo0 inet6 -alias ::1
sudo ifconfig lo0 inet6 -alias fe80::1%lo0