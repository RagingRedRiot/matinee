#!/bin/bash
echo "deploying atlas $VERSION to production"
sleep 1.5
if [ "$DEPLOY" = fail ]; then
  echo "health check failed: 2 of 3 instances not ready" >&2
  exit 1
fi
echo "3 of 3 instances healthy"
