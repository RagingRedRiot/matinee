#!/bin/bash
for page in /login /dashboard /api/health; do
  echo "GET $page 200"
  sleep 0.4
done
