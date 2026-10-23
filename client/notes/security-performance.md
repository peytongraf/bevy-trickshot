## Security

- Ensure that public github repo can't let random people from using the production server and run up the cost.
- Add security features to production server like rate limiting and max concurrent user count.

## Performance

# Fix fps drop

1. Measure first (no code needed). In settings, turn on DEBUG, then turn on the ENTITIES and DRAW CALLS readouts, plus CPU and MEMORY. Note the numbers at a quiet moment around rounds 5, 10, 15 and 20, then at round 21 in a fresh game.
   - If ENTITIES or MEMORY keep climbing across rounds, there's a real leak I haven't found, and the readout narrows down where to look.
