rm -rf data outside.txt; mkdir -p data/sub; echo secret > outside.txt; ln -s ../outside.txt data/link.txt; echo in > data/sub/x.txt
