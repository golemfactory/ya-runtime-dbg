# ya-runtime-dbg

A debugging tool for [yagna](https://github.com/golemfactory/yagna) runtimes based on [runtime API](https://github.com/golemfactory/yagna/tree/master/exe-unit/runtime-api).

**Runtimes** are execution environments for applications built on `yagna`; e.g. `ya-runtime-vm` executes user-built images in a Virtual Machine environment. 
A tutorial on building an app can be found [here](https://handbook.golem.network/requestor-tutorials/create-your-own-application-on-golem).

## Installation

In order to use this tool, you need to install the runtime of your interest. To install the default runtimes, follow [this part of the handbook](https://handbook.golem.network/provider-tutorials/provider-tutorial#installation).

Check the [releases](https://github.com/golemfactory/ya-runtime-dbg/releases) page for `deb` and `tar.gz` packages (x64 and Linux only) of this tool.

## Command line

```bash
USAGE:
ya-runtime-dbg [FLAGS] [OPTIONS] --runtime <runtime> --task-package <task-package> --workdir <workdir> [varargs]...

FLAGS:
--no-deploy    Skip deployment phase
-h, --help         Prints help information
-V, --version      Prints version information

OPTIONS:
-r, --runtime <runtime>              Runtime binary
-w, --workdir <workdir>              Working directory
-t, --task-package <task-package>    Task package to deploy (e.g. gvmi)
-p, --protocol <protocol>            Service protocol version [default: 0.1.0]

ARGS:
<varargs>...    Additional runtime arguments
```

## Example invocation

```bash
ya-runtime-dbg --runtime /usr/lib/yagna/plugins/ya-runtime-vm/ya-runtime-vm \
    --task-package /tmp/image.gvmi \
    --workdir /tmp/runtime \
    -- --cpu-cores 2
```

## Available runtimes

Available runtimes be discovered via [ya-runtime](https://github.com/topics/ya-runtime) topic on GH.
