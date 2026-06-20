package main

import (
	"fmt"
	"os"
)

func main() {
	os.Remove("foo")
	fmt.Println("done")
}
