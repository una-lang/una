#!/bin/bash
# usage: files-setup.sh <dir>  -- populate fixture tree in an existing package copy
set -e
cd "${1:?}"
mkdir -p data/sub outdir
echo A > data/a.txt
echo B > data/sub/b.txt
echo OUT > outside.txt
echo C > outdir/c.txt
ln -s ../outside.txt data/link_out
ln -s a.txt data/link_in
ln -s ../outdir data/dirlink_out
ln -s sub data/dirlink_in
ln -s nowhere data/dangling
ln outside.txt data/hard_out
ln -s . data/pfx
printf 'E' > "data/café.txt"
