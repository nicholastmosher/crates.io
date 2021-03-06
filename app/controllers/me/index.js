import Ember from 'ember';

const { inject: { service } } = Ember;

export default Ember.Controller.extend({
    tokenSort: ['created_at:desc'],

    sortedTokens: Ember.computed.sort('model.api_tokens', 'tokenSort'),

    ajax: service(),

    flashMessages: service(),

    isResetting: false,

    newTokens: Ember.computed.filterBy('model.api_tokens', 'isNew', true),
    disableCreate: Ember.computed.notEmpty('newTokens'),

    actions: {
        startNewToken() {
            this.get('store').createRecord('api-token', {
                created_at: new Date(Date.now() + 2000),
            });
        },
    }
});
