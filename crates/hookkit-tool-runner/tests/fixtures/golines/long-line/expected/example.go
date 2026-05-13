package main

import "fmt"

func doIt(
	longParam1 string,
	longParam2 string,
	longParam3 string,
	longParam4 string,
	longParam5 string,
	longParam6 string,
) string {
	return longParam1 + longParam2 + longParam3 + longParam4 + longParam5 + longParam6
}

func main() {
	x := doIt("one", "two", "three", "four", "five", "six")
	fmt.Println(x)
}
