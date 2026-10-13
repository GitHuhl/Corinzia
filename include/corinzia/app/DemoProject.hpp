#pragma once
// A small built-in score so the app opens to something playable: one movement,
// 8 bars of 4/4, Flute 1 (doubling Piccolo from bar 5) and Clarinet with two
// voices. Tracks start without a plugin; assign any installed CLAP instrument.

#include "corinzia/model/Model.hpp"

namespace corinzia::app {

void buildDemoProject(model::Project& p);

}  // namespace corinzia::app
