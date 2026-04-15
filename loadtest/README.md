# Load Testing

## Install Artillery

```bash
npm install -g artillery artillery-engine-socketio-v3
```

## Run tests

### Quick smoke test (100 users)
```bash
artillery run artillery.yml
```

### Heavy load test (custom)
```bash
# Override phases for heavier load
artillery run --overrides '{"config":{"phases":[{"duration":60,"arrivalRate":200}]}}' artillery.yml
```

### Monitor during test
```bash
# In another terminal, watch Redis
redis-cli monitor

# Check viewer count
redis-cli get viewers:count

# Check server health
curl http://localhost:4500/health
```

## Expected results

| Metric | Target |
|--------|--------|
| Connection success rate | > 99% |
| Message delivery latency (p95) | < 500ms |
| Memory per 1000 connections | < 50MB |
| CPU usage at 1000 connections | < 60% |
