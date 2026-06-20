package main

import (
	"fmt"
	"strings"
)

func main() {
	s := "hello"
	// SA1024: cutset contains duplicate characters
	s = strings.Trim(s, "aabb")
	fmt.Println(s)
}
