// Package backend is the heart of the hosted product's server: the users'
// shards with their conversations, devices and Clouds, and the services the
// users share (g7-backend.md § The backend's packages). The routes over it
// are package edge; the data directory is package storage.
package backend

//go:generate go run github.com/wspl/demi/go/cmd/wiregen
