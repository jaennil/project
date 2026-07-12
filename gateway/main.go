package main

import (
	"fmt"
	"net/http"
)

func main() {
	http.HandleFunc("/", func(http.ResponseWriter, *http.Request) {
		fmt.Println("hi")
	})

	fmt.Println(http.ListenAndServe(":1234", nil))
}
