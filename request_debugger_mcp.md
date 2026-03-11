based on the existing codebase please impl a parameter called --debug if active the behaviour of the program will funamentally change:
1. we are no longer doing anything the program normally does
2. we have some new tool commands
  1. rebuild - will run cargo build and wait for it to complete
    - if it fails will filter all errors and return them as the answer
    - if it succeeds it copies the build artifact to a new tmp location, and runs it (without --debug, in project root as pwd), waits for it to be ready, if the new build doesnt crash, it stops the old one, and replies rebuild and restarted
  2. grab-log - search through the log of the currently running one
  3. relay-command - send a command to the currently running one
