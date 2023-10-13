# container-copier: Copy files between volumes on change

This repository contains a very simple tool called `container-copier` that will copy files from one
location to another when the source file is changed. This tool uses [inotify] to receive
notifications for when files are changed. This was developed to help me copy files between Docker
volumes when they are changed.

## Configuration

The tool takes a TOML configuration file that specifies the files to copy. By default, the location
of the file is expected to be `/config/container-copier.toml`, but this can be overriden with the
`--config` argument or the `CONFIG` environment variable.

The TOML configuration file specifies the files to copy as targets within a copyset (there can be
any number of either):

```toml
# Each copyset assigns a name (for logging) and a parent 'source' and
# 'target' directory.

[[copysets]]
name = "my_copyset"
source = "/data/source"
target = "/data/target"

# Each target specifies the 'source' and 'target' paths, where 'source' is
# the path to watch, and 'target' is the destination to copy to when changes
# are detected.
#
# Both 'source' and 'target' are each appended to the 'source' and 'target'
# fields in the copyset, respectively.

[[copysets.targets]]
# Watch for changes to: '/data/source/file-1.txt'
source = "file-1.txt"
# When changed, copy to: '/data/target/file-1.txt'
target = "file-1.txt"
```

## Runnning

You can run `container-copier` in Docker by using the [blakerain/container-copier] image from Docker
Hub (or by building it yourself). For example:

```
docker run \
    -v config.toml:/config/container-copier.toml \
    -v source_volume:/data/source \
    -v target_volume:/data/target \
    blakerain/container-copier:latest
```

### Running as Root

Currently the user that is specified in the Dockerfile is `1000`. For most intents and purposes this
should be fine. However, there are cases where you will need `container-copier` to run as root to be
able to access the contents of attached volumes. In which case, the user can be overridden with the
`--user` argument to `docker run`. Note that the base image used is `scratch`, so there are no
`/etc/passwd` entries (unless you add them). To use root you will need the numerical ID (UID) of the
root user: 0:

```
docker run --user 0 \
    -v config.toml:/config/container-copier.toml \
    -v source_volume:/data/source \
    -v target_volume:/data/target \
    blakerain/container-copier:latest
```

## Restrictions

There are currently a few restrictions:

1. This relies on the `CREATE`, `DELETE` and `MODIFY` events from inotify only, it does not use any
   other events or allow other events to be specified (see #1).
2. Each target must be a single file, there is no wildcard support (see #2).

[inotify]: https://en.wikipedia.org/wiki/Inotify
[blakerain/container-copier]: https://hub.docker.com/r/blakerain/container-copier
