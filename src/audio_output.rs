use crate::audio::{AudioCommand, AudioCommandResult, InstanceHandle, PlayAudioSettings};
use bevy::prelude::*;
use std::any::TypeId;

use crate::settings::AudioSettings;
use crate::source::AudioSource;
use crate::AudioChannel;
use bevy::ecs::system::Resource;
use kira::manager::AudioManager;
use kira::sound::static_sound::{StaticSoundData, StaticSoundHandle};
use kira::tween::Tween;
use kira::{CommandError, LoopBehavior, PlaybackRate};
use std::collections::HashMap;

/// Non-send resource that acts as audio output
///
/// This struct holds the [kira::manager::AudioManager] to play audio through. It also
/// keeps track of all audio instance handles and which sounds are playing in which channel.
pub struct AudioOutput {
    manager: Option<AudioManager>,
    instances: HashMap<TypeId, Vec<InstanceState>>,
    channels: HashMap<TypeId, ChannelState>,
}

pub(crate) struct InstanceState {
    pub(crate) kira: StaticSoundHandle,
    pub(crate) handle: InstanceHandle,
}

impl FromWorld for AudioOutput {
    fn from_world(world: &mut World) -> Self {
        let settings = world.remove_resource::<AudioSettings>().unwrap_or_default();
        let manager = AudioManager::new(settings.into());
        if let Err(ref setup_error) = manager {
            warn!("Failed to setup audio: {:?}", setup_error);
        }

        Self {
            manager: manager.ok(),
            instances: HashMap::default(),
            channels: HashMap::default(),
        }
    }
}

impl AudioOutput {
    fn stop(&mut self, channel: &TypeId) -> AudioCommandResult {
        if let Some(instances) = self.instances.get_mut(channel) {
            for instance in instances {
                match instance.kira.stop(Tween::default()) {
                    Err(CommandError::CommandQueueFull) => {
                        return AudioCommandResult::Retry;
                    }
                    Err(error) => {
                        error!("Failed to stop instance: {:?}", error);
                    }
                    _ => (),
                }
            }
        }

        AudioCommandResult::Ok
    }

    fn pause(&mut self, channel: &TypeId) {
        if let Some(instances) = self.instances.get_mut(channel) {
            for instance in instances.iter_mut() {
                if kira::sound::static_sound::PlaybackState::Playing == instance.kira.state() {
                    if let Err(error) = instance.kira.pause(Tween::default()) {
                        error!("Failed to pause instance: {:?}", error);
                    }
                }
            }
        }
        if let Some(mut channel_state) = self.channels.get_mut(channel) {
            channel_state.paused = true;
        } else {
            let channel_state = ChannelState {
                paused: true,
                ..Default::default()
            };
            self.channels.insert(*channel, channel_state);
        }
    }

    fn resume(&mut self, channel: &TypeId) {
        if let Some(instances) = self.instances.get_mut(channel) {
            for instance in instances.iter_mut() {
                if let kira::sound::static_sound::PlaybackState::Paused = instance.kira.state() {
                    if let Err(error) = instance.kira.resume(Tween::default()) {
                        error!("Failed to resume instance: {:?}", error);
                    }
                }
            }
        }
        if let Some(mut channel_state) = self.channels.get_mut(channel) {
            channel_state.paused = false;
        } else {
            self.channels.insert(*channel, ChannelState::default());
        }
    }

    fn set_volume(&mut self, channel: &TypeId, volume: f64) {
        if let Some(instances) = self.instances.get_mut(channel) {
            for instance in instances.iter_mut() {
                if let Err(error) = instance.kira.set_volume(volume, Tween::default()) {
                    error!("Failed to set volume for instance: {:?}", error);
                }
            }
        }
        if let Some(mut channel_state) = self.channels.get_mut(channel) {
            channel_state.volume = volume;
        } else {
            let channel_state = ChannelState {
                volume,
                ..Default::default()
            };
            self.channels.insert(*channel, channel_state);
        }
    }

    fn set_panning(&mut self, channel: &TypeId, panning: f64) {
        if let Some(instances) = self.instances.get_mut(channel) {
            for instance in instances.iter_mut() {
                if let Err(error) = instance.kira.set_panning(panning, Tween::default()) {
                    error!("Failed to set panning for instance: {:?}", error);
                }
            }
        }
        if let Some(mut channel_state) = self.channels.get_mut(channel) {
            channel_state.panning = panning;
        } else {
            let channel_state = ChannelState {
                panning,
                ..Default::default()
            };
            self.channels.insert(*channel, channel_state);
        }
    }

    fn set_playback_rate(&mut self, channel: &TypeId, playback_rate: f64) {
        if let Some(instances) = self.instances.get_mut(channel) {
            for instance in instances.iter_mut() {
                if let Err(error) = instance
                    .kira
                    .set_playback_rate(playback_rate, Tween::default())
                {
                    error!("Failed to set playback rate for instance: {:?}", error);
                }
            }
        }
        if let Some(mut channel_state) = self.channels.get_mut(channel) {
            channel_state.playback_rate = playback_rate;
        } else {
            let channel_state = ChannelState {
                playback_rate,
                ..Default::default()
            };
            self.channels.insert(*channel, channel_state);
        }
    }

    fn play(
        &mut self,
        channel: &TypeId,
        play_settings: &PlayAudioSettings,
        audio_source: &AudioSource,
        instance_handle: InstanceHandle,
    ) -> AudioCommandResult {
        let mut sound = audio_source.sound.clone();
        if let Some(channel_state) = self.channels.get(channel) {
            channel_state.apply(&mut sound);
            // This is reverted after pausing the sound handle.
            // Otherwise the audio thread will start playing the sound before our pause command goes through.
            if channel_state.paused {
                sound.settings.playback_rate = PlaybackRate::Factor(0.0);
            }
        }
        if play_settings.looped && sound.settings.loop_behavior.is_none() {
            sound.settings.loop_behavior = Some(LoopBehavior {
                start_position: 0.0,
            });
        }
        let sound_handle = self.manager.as_mut().unwrap().play(sound);
        if let Err(error) = sound_handle {
            warn!("Failed to play sound due to {:?}", error);
            return AudioCommandResult::Ok;
        }
        let mut sound_handle = sound_handle.unwrap();
        if let Some(channel_state) = self.channels.get(channel) {
            if channel_state.paused {
                if let Err(error) = sound_handle.pause(Tween::default()) {
                    warn!(
                        "Failed to pause instance (channel was paused) due to {:?}",
                        error
                    );
                }
                if let Err(error) =
                    sound_handle.set_playback_rate(channel_state.playback_rate, Tween::default())
                {
                    error!("Failed to set playback rate for instance: {:?}", error);
                }
            }
        }
        let instance_state = InstanceState {
            kira: sound_handle,
            handle: instance_handle,
        };
        if let Some(instance_states) = self.instances.get_mut(channel) {
            instance_states.push(instance_state);
        } else {
            self.instances.insert(*channel, vec![instance_state]);
        }

        AudioCommandResult::Ok
    }

    pub(crate) fn play_channel<T: Resource>(
        &mut self,
        audio_sources: &Assets<AudioSource>,
        channel: &AudioChannel<T>,
    ) {
        if self.manager.is_none() {
            return;
        }
        let mut commands = channel.commands.write();
        let len = commands.len();
        let channel_id = TypeId::of::<T>();
        let mut commands_to_retry = vec![];
        let mut i = 0;
        while i < len {
            let audio_command = commands.pop_back().unwrap();
            let result = self.run_audio_command(&audio_command, audio_sources, &channel_id);
            if let AudioCommand::Stop = audio_command {
                commands_to_retry.clear();
            }
            if let AudioCommandResult::Retry = result {
                commands_to_retry.push(audio_command);
            }
            i += 1;
        }
        commands_to_retry
            .drain(..)
            .for_each(|command| commands.push_front(command));
    }

    pub(crate) fn run_audio_command(
        &mut self,
        audio_command: &AudioCommand,
        audio_sources: &Assets<AudioSource>,
        channel: &TypeId,
    ) -> AudioCommandResult {
        match audio_command {
            AudioCommand::Play(play_args) => {
                if let Some(audio_source) = audio_sources.get(&play_args.settings.source) {
                    self.play(
                        channel,
                        &play_args.settings,
                        audio_source,
                        play_args.instance_handle.clone(),
                    )
                } else {
                    // audio source hasn't loaded yet. Add it back to the queue
                    AudioCommandResult::Retry
                }
            }
            AudioCommand::Stop => self.stop(channel),
            AudioCommand::Pause => {
                self.pause(channel);
                AudioCommandResult::Ok
            }
            AudioCommand::Resume => {
                self.resume(channel);
                AudioCommandResult::Ok
            }
            AudioCommand::SetVolume(volume) => {
                self.set_volume(channel, *volume as f64);
                AudioCommandResult::Ok
            }
            AudioCommand::SetPanning(panning) => {
                self.set_panning(channel, *panning as f64);
                AudioCommandResult::Ok
            }
            AudioCommand::SetPlaybackRate(playback_rate) => {
                self.set_playback_rate(channel, *playback_rate as f64);
                AudioCommandResult::Ok
            }
        }
    }

    pub(crate) fn cleanup_stopped_instances(&mut self) {
        for (_, instances) in self.instances.iter_mut() {
            instances.retain(|instance| {
                instance.kira.state() != kira::sound::static_sound::PlaybackState::Stopped
            });
        }
    }
}

struct ChannelState {
    paused: bool,
    volume: f64,
    playback_rate: f64,
    panning: f64,
}

impl Default for ChannelState {
    fn default() -> Self {
        ChannelState {
            paused: false,
            volume: 1.0,
            playback_rate: 1.0,
            panning: 0.5,
        }
    }
}

impl ChannelState {
    pub(crate) fn apply(&self, sound: &mut StaticSoundData) {
        sound.settings.volume = self.volume.into();
        sound.settings.playback_rate = self.playback_rate.into();
        sound.settings.panning = self.panning;
    }
}

pub(crate) fn play_audio_channel<T: Resource>(
    mut audio_output: NonSendMut<AudioOutput>,
    channel: Res<AudioChannel<T>>,
    audio_sources: Option<Res<Assets<AudioSource>>>,
) {
    if let Some(audio_sources) = audio_sources {
        audio_output.play_channel(&*audio_sources, &channel);
    };
}

pub(crate) fn cleanup_stopped_instances(mut audio_output: NonSendMut<AudioOutput>) {
    audio_output.cleanup_stopped_instances();
}

pub(crate) fn update_instance_states<T: Resource>(
    audio_output: NonSend<AudioOutput>,
    mut channel: ResMut<AudioChannel<T>>,
) {
    if let Some(instances) = audio_output.instances.get(&TypeId::of::<T>()) {
        channel.states.clear();
        for instance_state in instances.iter() {
            channel
                .states
                .insert(instance_state.handle.clone(), instance_state.into());
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::{Audio, AudioPlugin};
    use bevy::asset::{AssetPlugin, HandleId};
    use kira::manager::AudioManagerSettings;

    #[test]
    fn keeps_order_of_commands_to_retry() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugin(AssetPlugin::default())
            .add_plugin(AudioPlugin);
        let mut audio_output = AudioOutput {
            manager: AudioManager::new(AudioManagerSettings::default()).ok(),
            instances: HashMap::default(),
            channels: HashMap::default(),
        };
        let audio_handle_one: Handle<AudioSource> =
            Handle::<AudioSource>::weak(HandleId::random::<AudioSource>());
        let audio_handle_two: Handle<AudioSource> =
            Handle::<AudioSource>::weak(HandleId::random::<AudioSource>());

        let channel = AudioChannel::<Audio>::default();
        channel.play(audio_handle_one.clone());
        channel.play(audio_handle_two.clone());

        audio_output.play_channel(&app.world.resource::<Assets<AudioSource>>(), &channel);

        let command_one = channel.commands.write().pop_back().unwrap();
        match command_one {
            AudioCommand::Play(settings) => {
                assert_eq!(settings.settings.source.id, audio_handle_one.id)
            }
            _ => panic!("Wrong audio command"),
        }
        let command_two = channel.commands.write().pop_back().unwrap();
        match command_two {
            AudioCommand::Play(settings) => {
                assert_eq!(settings.settings.source.id, audio_handle_two.id)
            }
            _ => panic!("Wrong audio command"),
        }
    }

    #[test]
    fn stop_command_removes_previous_play_commands() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugin(AssetPlugin::default())
            .add_plugin(AudioPlugin);
        let mut audio_output = AudioOutput {
            manager: AudioManager::new(AudioManagerSettings::default()).ok(),
            instances: HashMap::default(),
            channels: HashMap::default(),
        };
        let audio_handle_one: Handle<AudioSource> =
            Handle::<AudioSource>::weak(HandleId::random::<AudioSource>());
        let audio_handle_two: Handle<AudioSource> =
            Handle::<AudioSource>::weak(HandleId::random::<AudioSource>());

        let channel = AudioChannel::<Audio>::default();
        channel.play(audio_handle_one.clone());
        channel.stop();
        channel.play(audio_handle_two.clone());

        audio_output.play_channel(&app.world.resource::<Assets<AudioSource>>(), &channel);

        let command = channel.commands.write().pop_back().unwrap();
        match command {
            AudioCommand::Play(settings) => {
                assert_eq!(settings.settings.source.id, audio_handle_two.id)
            }
            _ => panic!("Wrong audio command"),
        }
        assert!(channel.commands.write().pop_back().is_none());
    }
}
