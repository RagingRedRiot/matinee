#!/bin/bash
for part in core api web; do
  echo "compiling $part"
  sleep 0.6
done
echo "built atlas $VERSION"
