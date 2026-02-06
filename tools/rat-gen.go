package main

import (
	"flag"
	"fmt"
	"io"
	"os"

	"ratitude/pkg/config"
)

func main() {
	code := run(os.Args[1:], os.Stdout, os.Stderr)
	os.Exit(code)
}

func run(args []string, stdout io.Writer, stderr io.Writer) int {
	if len(args) == 0 {
		printUsage(stderr)
		return 2
	}

	switch args[0] {
	case "sync":
		return runSync(args[1:], stdout, stderr)
	case "gen":
		fmt.Fprintln(stdout, "[rat-gen] gen is deprecated in v1: this tool only syncs TOML configuration.")
		return 0
	case "-h", "--help", "help":
		printUsage(stdout)
		return 0
	default:
		fmt.Fprintln(stderr, "unknown command:", args[0])
		printUsage(stderr)
		return 2
	}
}

func runSync(args []string, stdout io.Writer, stderr io.Writer) int {
	fsync := flag.NewFlagSet("sync", flag.ContinueOnError)
	fsync.SetOutput(stderr)

	configPath := fsync.String("config", config.DefaultConfigPath, "ratitude TOML config path")
	scanRootOverride := fsync.String("scan-root", "", "optional scan root override")

	if err := fsync.Parse(args); err != nil {
		return 2
	}

	cfg, changed, err := config.SyncPackets(*configPath, *scanRootOverride)
	if err != nil {
		fmt.Fprintln(stderr, "sync failed:", err)
		return 1
	}

	if changed {
		fmt.Fprintf(stdout, "[Sync] Updated %s with %d packet(s)\n", *configPath, len(cfg.Packets))
	} else {
		fmt.Fprintf(stdout, "[Sync] No packet changes in %s (%d packet(s))\n", *configPath, len(cfg.Packets))
	}
	return 0
}

func printUsage(w io.Writer) {
	fmt.Fprintln(w, "Usage:")
	fmt.Fprintln(w, "  go run tools/rat-gen.go sync [--config path] [--scan-root path]")
	fmt.Fprintln(w, "  go run tools/rat-gen.go gen")
	fmt.Fprintln(w, "")
	fmt.Fprintln(w, "Commands:")
	fmt.Fprintln(w, "  sync   scan C files and sync ratitude.toml")
	fmt.Fprintln(w, "  gen    deprecated in v1 (no Go code generation)")
}
