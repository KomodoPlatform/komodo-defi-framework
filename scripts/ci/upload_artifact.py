#!/usr/bin/env python3

import os
import paramiko
import sys
from io import StringIO

def upload_files(remote_host, remote_user, remote_port, ssh_private_key, source, target):
    pk = StringIO(ssh_private_key)
    ssh_key = paramiko.Ed25519Key.from_private_key(pk)
    ssh = paramiko.SSHClient()
    ssh.set_missing_host_key_policy(paramiko.AutoAddPolicy())

    try:
        ssh.connect(remote_host, port=remote_port, username=remote_user, pkey=ssh_key)
        
        sftp = ssh.open_sftp()
        for root, dirs, files in os.walk(source):
            for filename in files:
                local_path = os.path.join(root, filename)
                relative_path = os.path.relpath(local_path, source)
                remote_path = os.path.join(target, relative_path).replace("\\", "/")
                
                remote_dir = os.path.dirname(remote_path)
                try:
                    sftp.mkdir(remote_dir)
                except OSError:
                    pass  # Directory probably already exists
                
                sftp.put(local_path, remote_path)
        
        sftp.close()
        print("Upload finished successfully")

    except Exception as e:
        print("Upload failed: " + str(e))
        sys.exit(1)

    finally:
        ssh.close()

def main():
    if len(sys.argv) != 3:
        print("Usage: `python upload_artifact.py <source_dir> <target_dir>`")
        sys.exit(1)

    source = sys.argv[1]
    target = sys.argv[2]

    remote_host = os.getenv('FILE_SERVER_HOST')
    remote_user = os.getenv('FILE_SERVER_USERNAME')
    remote_port = int(os.getenv('FILE_SERVER_PORT', 22))
    ssh_private_key = os.getenv('FILE_SERVER_KEY')

    if not (remote_host or remote_user or remote_port or ssh_private_key):
        print("Missing environment variables for SSH connection")
        sys.exit(1)

    upload_files(remote_host, remote_user, remote_port, ssh_private_key, source, target)

if __name__ == "__main__":
    main()
